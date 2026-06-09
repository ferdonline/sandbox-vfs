//! In-memory filesystem with a Rust-owned namespace and kernel-backed files.
//!
//! Regular-file contents live in anonymous memfds so callers can use ordinary
//! kernel `read`, `write`, `mmap`, and `ftruncate` operations on returned fds.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::{OsStr, OsString},
    os::unix::io::RawFd,
    path::{Component, Path},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};

use libc::{mode_t, stat, O_CREAT};

use crate::filesystem::{LowLevelFS, OpenResult, OpenedFile, VfsDirEntry, VfsEntryKind};

type NodeId = u64;
type NodeMap = Arc<RwLock<HashMap<NodeId, Arc<Node>>>>;
static NEXT_NODE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct MemoryFS {
    id: String,
    root: NodeId,
    nodes: NodeMap,
}

#[derive(Debug)]
struct Node {
    id: NodeId,
    mode: RwLock<mode_t>,
    kind: NodeKind,
}

#[derive(Debug)]
enum NodeKind {
    File {
        backing_fd: RawFd,
    },
    Directory {
        entries: RwLock<BTreeMap<OsString, NodeId>>,
        placeholder_fd: RawFd,
    },
}

#[derive(Debug)]
struct MemoryOpenedFile {
    node: Arc<Node>,
    nodes: NodeMap,
}

impl OpenedFile for MemoryOpenedFile {
    fn stat(&self, statbuf: &mut stat) -> i32 {
        stat_node(&self.node, &self.nodes, statbuf)
    }

    fn read_dir(&self) -> Option<Vec<VfsDirEntry>> {
        read_node_dir(&self.node, &self.nodes)
    }
}

impl Node {
    fn entry_kind(&self) -> VfsEntryKind {
        match self.kind {
            NodeKind::File { .. } => VfsEntryKind::File,
            NodeKind::Directory { .. } => VfsEntryKind::Dir,
        }
    }

    fn backing_fd(&self) -> RawFd {
        match self.kind {
            NodeKind::File { backing_fd } => backing_fd,
            NodeKind::Directory { placeholder_fd, .. } => placeholder_fd,
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Node {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.backing_fd());
        }
    }
}

/// Create an anonymous kernel file used as node backing.
#[cfg(target_os = "linux")]
fn create_memfd(name: &[u8]) -> RawFd {
    unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr().cast::<i8>(), 0) as RawFd }
}

#[cfg(not(target_os = "linux"))]
fn create_memfd(_name: &[u8]) -> RawFd {
    static NEXT_FD: AtomicU64 = AtomicU64::new(10_000);
    NEXT_FD.fetch_add(1, Ordering::Relaxed) as RawFd
}

/// Open a fresh file description for a node's anonymous backing file.
#[cfg(target_os = "linux")]
fn open_backing(backing_fd: RawFd, flags: i32) -> RawFd {
    let path = format!("/proc/self/fd/{backing_fd}\0");
    // These flags describe virtual namespace lookup, which has already happened.
    let flags = flags & !(libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_DIRECTORY);
    // Bypass libc because this function is itself used by the interposed open hook.
    unsafe {
        libc::syscall(
            libc::SYS_openat,
            libc::AT_FDCWD,
            path.as_ptr().cast::<i8>(),
            flags,
            0,
        ) as RawFd
    }
}

#[cfg(not(target_os = "linux"))]
fn open_backing(_backing_fd: RawFd, _flags: i32) -> RawFd {
    static NEXT_FD: AtomicU64 = AtomicU64::new(20_000);
    NEXT_FD.fetch_add(1, Ordering::Relaxed) as RawFd
}

impl MemoryFS {
    pub fn new(id: impl Into<String>) -> Box<Self> {
        let root_id = NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed);
        let root = Arc::new(Node {
            id: root_id,
            mode: RwLock::new(0o755),
            kind: NodeKind::Directory {
                entries: RwLock::new(BTreeMap::new()),
                placeholder_fd: create_memfd(b"sandbox-vfs-dir\0"),
            },
        });
        let mut nodes = HashMap::new();
        nodes.insert(root.id, root);

        Box::new(Self {
            id: id.into(),
            root: root_id,
            nodes: Arc::new(RwLock::new(nodes)),
        })
    }

    fn assert_absolute(path: &Path) {
        assert!(
            path.is_absolute(),
            "MemoryFS backends only accept absolute paths, got {path:?}"
        );
    }

    fn node(&self, id: NodeId) -> Option<Arc<Node>> {
        self.nodes.read().unwrap().get(&id).cloned()
    }

    fn resolve(&self, path: &Path) -> Option<Arc<Node>> {
        Self::assert_absolute(path);
        let mut node = self.node(self.root)?;

        for component in path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            let NodeKind::Directory { entries, .. } = &node.kind else {
                return None;
            };
            let child_id = *entries.read().unwrap().get(name)?;
            node = self.node(child_id)?;
        }
        Some(node)
    }

    fn resolve_parent<'a>(&self, path: &'a Path) -> Option<(Arc<Node>, &'a OsStr)> {
        Self::assert_absolute(path);
        Some((self.resolve(path.parent()?)?, path.file_name()?))
    }

    fn create_node(&self, path: &Path, kind: VfsEntryKind, mode: mode_t) -> Option<Arc<Node>> {
        let (parent, name) = self.resolve_parent(path)?;
        let NodeKind::Directory { entries, .. } = &parent.kind else {
            return None;
        };

        let mut entries = entries.write().unwrap();
        if entries.contains_key(name) {
            return None;
        }

        let id = NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed);
        let node_kind = match kind {
            VfsEntryKind::File => NodeKind::File {
                backing_fd: create_memfd(b"sandbox-vfs-file\0"),
            },
            VfsEntryKind::Dir => NodeKind::Directory {
                entries: RwLock::new(BTreeMap::new()),
                placeholder_fd: create_memfd(b"sandbox-vfs-dir\0"),
            },
        };
        if node_kind.backing_fd() < 0 {
            return None;
        }

        let node = Arc::new(Node {
            id,
            mode: RwLock::new(mode & 0o7777),
            kind: node_kind,
        });
        self.nodes.write().unwrap().insert(id, node.clone());
        entries.insert(name.to_os_string(), id);
        Some(node)
    }

    fn open_node(&self, path: &Path, flags: i32, mode: mode_t) -> (RawFd, Option<Arc<Node>>) {
        Self::assert_absolute(path);
        let existing = self.resolve(path);
        if existing.is_some()
            && flags & (libc::O_CREAT | libc::O_EXCL) == libc::O_CREAT | libc::O_EXCL
        {
            return (-1, None);
        }

        let node = match existing {
            Some(node) => node,
            None if flags & O_CREAT != 0 => {
                let Some(node) = self.create_node(path, VfsEntryKind::File, mode) else {
                    return (-1, None);
                };
                node
            }
            None => return (-1, None),
        };

        if flags & libc::O_DIRECTORY != 0 && !matches!(node.kind, NodeKind::Directory { .. }) {
            return (-1, None);
        }

        let fd = open_backing(node.backing_fd(), flags);
        if fd < 0 {
            return (fd, None);
        }
        (fd, Some(node))
    }
}

impl NodeKind {
    fn backing_fd(&self) -> RawFd {
        match self {
            Self::File { backing_fd } => *backing_fd,
            Self::Directory { placeholder_fd, .. } => *placeholder_fd,
        }
    }
}

fn node_from_map(nodes: &NodeMap, id: NodeId) -> Option<Arc<Node>> {
    nodes.read().unwrap().get(&id).cloned()
}

fn stat_node(node: &Node, nodes: &NodeMap, statbuf: &mut stat) -> i32 {
    *statbuf = unsafe { std::mem::zeroed() };
    if matches!(node.kind, NodeKind::File { .. }) {
        unsafe {
            libc::fstat(node.backing_fd(), statbuf);
        }
    }
    statbuf.st_mode = match node.kind {
        NodeKind::File { .. } => libc::S_IFREG,
        NodeKind::Directory { .. } => libc::S_IFDIR,
    } | *node.mode.read().unwrap();
    statbuf.st_ino = node.id as _;
    statbuf.st_nlink = match node.kind {
        NodeKind::File { .. } => 1,
        NodeKind::Directory { ref entries, .. } => {
            2 + entries
                .read()
                .unwrap()
                .values()
                .filter(|id| {
                    node_from_map(nodes, **id)
                        .is_some_and(|node| matches!(node.kind, NodeKind::Directory { .. }))
                })
                .count() as libc::nlink_t
        }
    };
    statbuf.st_blksize = 4096;
    0
}

fn read_node_dir(node: &Node, nodes: &NodeMap) -> Option<Vec<VfsDirEntry>> {
    let NodeKind::Directory { entries, .. } = &node.kind else {
        return None;
    };

    entries
        .read()
        .unwrap()
        .iter()
        .map(|(name, id)| {
            let node = node_from_map(nodes, *id)?;
            Some(VfsDirEntry {
                name: name.clone(),
                kind: node.entry_kind(),
                ino: Some(node.id),
            })
        })
        .collect()
}

impl LowLevelFS for MemoryFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, path: &Path, _mode: i32) -> i32 {
        self.resolve(path).map_or(-1, |_| 0)
    }

    fn open(&self, path: &Path, flags: i32, mode: mode_t) -> RawFd {
        self.open_node(path, flags, mode).0
    }

    fn open_with_handle(&self, path: &Path, flags: i32, mode: mode_t) -> OpenResult {
        let (fd, node) = self.open_node(path, flags, mode);
        OpenResult {
            fd,
            opened: node.map(|node| {
                Arc::new(MemoryOpenedFile {
                    node,
                    nodes: self.nodes.clone(),
                }) as Arc<dyn OpenedFile>
            }),
        }
    }

    fn openat(&self, _dirfd: i32, path: &Path, flags: i32, mode: mode_t) -> i32 {
        Self::assert_absolute(path);
        self.open(path, flags, mode)
    }

    fn mkdir(&self, path: &Path, mode: mode_t) -> i32 {
        self.create_node(path, VfsEntryKind::Dir, mode)
            .map_or(-1, |_| 0)
    }

    fn chmod(&self, path: &Path, mode: mode_t) -> i32 {
        let Some(node) = self.resolve(path) else {
            return -1;
        };
        *node.mode.write().unwrap() = mode & 0o7777;
        if matches!(node.kind, NodeKind::File { .. }) {
            unsafe {
                libc::fchmod(node.backing_fd(), mode);
            }
        }
        0
    }

    fn stat(&self, path: &Path, statbuf: &mut stat) -> i32 {
        let Some(node) = self.resolve(path) else {
            return -1;
        };
        stat_node(&node, &self.nodes, statbuf)
    }

    fn read_dir(&self, path: &Path) -> Option<Vec<VfsDirEntry>> {
        let node = self.resolve(path)?;
        read_node_dir(&node, &self.nodes)
    }
}

#[cfg(test)]
mod test {
    use libc::{F_OK, O_CREAT};
    #[cfg(target_os = "linux")]
    use libc::{O_RDONLY, O_RDWR};

    use super::*;

    #[test]
    fn root_exists_by_default() {
        let fs = MemoryFS::new("memory");
        assert_eq!(fs.access(Path::new("/"), F_OK), 0);
    }

    #[test]
    fn creation_requires_existing_directory_parent() {
        let fs = MemoryFS::new("memory");
        assert_ne!(fs.open(Path::new("/missing/file"), O_CREAT, 0o644), 0);
        assert_eq!(fs.mkdir(Path::new("/dir"), 0o755), 0);
        assert!(fs.open(Path::new("/dir/file"), O_CREAT, 0o644) > 0);
    }

    #[test]
    fn stable_inode_is_independent_of_path_hashing() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/file"), O_CREAT, 0o640) > 0);
        let mut statbuf = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/file"), &mut statbuf), 0);
        let inode = statbuf.st_ino;
        assert_ne!(inode, 0);
        assert_eq!(fs.stat(Path::new("/file"), &mut statbuf), 0);
        assert_eq!(statbuf.st_ino, inode);
        assert_eq!(statbuf.st_mode & 0o7777, 0o640);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn independent_opens_have_independent_offsets_and_shared_contents() {
        let fs = MemoryFS::new("memory");
        let first = fs.open(Path::new("/file"), O_CREAT | O_RDWR, 0o644);
        assert_eq!(
            unsafe { libc::write(first, b"hello".as_ptr().cast(), 5) },
            5
        );
        unsafe {
            libc::close(first);
        }

        let left = fs.open(Path::new("/file"), O_RDONLY, 0);
        let right = fs.open(Path::new("/file"), O_RDONLY, 0);
        let mut buf = [0_u8; 3];
        assert_eq!(unsafe { libc::read(left, buf.as_mut_ptr().cast(), 2) }, 2);
        assert_eq!(&buf[..2], b"he");
        assert_eq!(unsafe { libc::read(right, buf.as_mut_ptr().cast(), 2) }, 2);
        assert_eq!(&buf[..2], b"he");

        unsafe {
            libc::close(left);
            libc::close(right);
        }
    }

    #[test]
    fn directory_entries_expose_child_inode() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/file"), O_CREAT, 0o644) > 0);
        let entries = fs.read_dir(Path::new("/")).unwrap();
        assert_eq!(entries[0].name, "file");
        let mut statbuf = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/file"), &mut statbuf), 0);
        assert_eq!(entries[0].ino, Some(statbuf.st_ino));
    }

    #[test]
    #[should_panic(expected = "MemoryFS backends only accept absolute paths")]
    fn rejects_relative_paths() {
        MemoryFS::new("memory").open(Path::new("file"), O_CREAT, 0o644);
    }
}
