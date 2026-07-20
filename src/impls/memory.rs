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
    parent: RwLock<Option<NodeId>>,
    mode: RwLock<mode_t>,
    links: RwLock<u64>,
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

    fn open_child(&self, path: &Path, flags: i32, mode: mode_t) -> Option<OpenResult> {
        let (fd, node) = open_node_relative(&self.node, &self.nodes, path, flags, mode)?;
        Some(OpenResult {
            fd,
            opened: node.map(|node| {
                Arc::new(MemoryOpenedFile {
                    node,
                    nodes: self.nodes.clone(),
                }) as Arc<dyn OpenedFile>
            }),
        })
    }

    fn mkdir_child(&self, path: &Path, mode: mode_t) -> Option<i32> {
        if !can_resolve_relative_to_opened_node(path) {
            return None;
        }

        Some(
            create_relative_node(&self.node, &self.nodes, path, VfsEntryKind::Dir, mode)
                .map_or(-1, |_| 0),
        )
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
            parent: RwLock::new(Some(root_id)),
            mode: RwLock::new(0o755),
            links: RwLock::new(1),
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
        create_node_in_dir(&parent, &self.nodes, name, kind, mode)
    }

    fn open_node(&self, path: &Path, flags: i32, mode: mode_t) -> (RawFd, Option<Arc<Node>>) {
        Self::assert_absolute(path);
        let existing = self.resolve(path);
        open_resolved_node(
            existing,
            || self.create_node(path, VfsEntryKind::File, mode),
            flags,
        )
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

fn resolve_from_root(path: &Path, nodes: &NodeMap, root: NodeId) -> Option<Arc<Node>> {
    MemoryFS::assert_absolute(path);
    let mut node = node_from_map(nodes, root)?;

    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let NodeKind::Directory { entries, .. } = &node.kind else {
            return None;
        };
        let child_id = *entries.read().unwrap().get(name)?;
        node = node_from_map(nodes, child_id)?;
    }
    Some(node)
}

fn resolve_parent_from_root<'a>(
    path: &'a Path,
    nodes: &NodeMap,
    root: NodeId,
) -> Option<(Arc<Node>, &'a OsStr)> {
    MemoryFS::assert_absolute(path);
    Some((
        resolve_from_root(path.parent()?, nodes, root)?,
        path.file_name()?,
    ))
}

fn create_node_in_dir(
    parent: &Node,
    nodes: &NodeMap,
    name: &OsStr,
    kind: VfsEntryKind,
    mode: mode_t,
) -> Option<Arc<Node>> {
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
        parent: RwLock::new(Some(parent.id)),
        mode: RwLock::new(mode & 0o7777),
        links: RwLock::new(1),
        kind: node_kind,
    });
    nodes.write().unwrap().insert(id, node.clone());
    entries.insert(name.to_os_string(), id);
    Some(node)
}

fn remove_node_at(path: &Path, nodes: &NodeMap, root: NodeId, kind: VfsEntryKind) -> i32 {
    MemoryFS::assert_absolute(path);
    if path == Path::new("/") {
        return -1;
    }

    let Some((parent, name)) = resolve_parent_from_root(path, nodes, root) else {
        return -1;
    };
    let NodeKind::Directory { entries, .. } = &parent.kind else {
        return -1;
    };

    let mut entries = entries.write().unwrap();
    let Some(id) = entries.get(name).copied() else {
        return -1;
    };
    let Some(node) = node_from_map(nodes, id) else {
        entries.remove(name);
        return -1;
    };

    match (&node.kind, kind) {
        (NodeKind::File { .. }, VfsEntryKind::File) => {}
        (NodeKind::Directory { entries, .. }, VfsEntryKind::Dir)
            if entries.read().unwrap().is_empty() => {}
        _ => return -1,
    }

    entries.remove(name);
    *node.parent.write().unwrap() = None;
    *node.links.write().unwrap() = 0;
    nodes.write().unwrap().remove(&id);
    0
}

fn rename_node_at(old_path: &Path, new_path: &Path, nodes: &NodeMap, root: NodeId) -> i32 {
    MemoryFS::assert_absolute(old_path);
    MemoryFS::assert_absolute(new_path);
    if old_path == Path::new("/") || new_path == Path::new("/") {
        return -1;
    }
    if old_path == new_path {
        return 0;
    }

    let Some((old_parent, old_name)) = resolve_parent_from_root(old_path, nodes, root) else {
        return -1;
    };
    let Some((new_parent, new_name)) = resolve_parent_from_root(new_path, nodes, root) else {
        return -1;
    };

    let NodeKind::Directory {
        entries: old_entries,
        ..
    } = &old_parent.kind
    else {
        return -1;
    };
    let NodeKind::Directory {
        entries: new_entries,
        ..
    } = &new_parent.kind
    else {
        return -1;
    };

    if old_parent.id == new_parent.id {
        let mut entries = old_entries.write().unwrap();
        let Some(old_id) = entries.get(old_name).copied() else {
            return -1;
        };
        let Some(node) = node_from_map(nodes, old_id) else {
            return -1;
        };
        if matches!(node.kind, NodeKind::Directory { .. }) && new_path.starts_with(old_path) {
            return -1;
        }
        if entries.get(new_name).copied() == Some(old_id) {
            return 0;
        }
        if let Some(replaced_id) = entries.get(new_name).copied() {
            let Some(replaced) = node_from_map(nodes, replaced_id) else {
                return -1;
            };
            if !can_replace_node(&node, &replaced) {
                return -1;
            }
            *replaced.parent.write().unwrap() = None;
            *replaced.links.write().unwrap() = 0;
            nodes.write().unwrap().remove(&replaced_id);
        }
        entries.remove(old_name);
        entries.insert(new_name.to_os_string(), old_id);
        return 0;
    }

    let old_id = {
        let entries = old_entries.read().unwrap();
        let Some(old_id) = entries.get(old_name).copied() else {
            return -1;
        };
        old_id
    };
    let Some(node) = node_from_map(nodes, old_id) else {
        return -1;
    };
    if matches!(node.kind, NodeKind::Directory { .. }) && new_path.starts_with(old_path) {
        return -1;
    }

    let replaced_id = {
        let entries = new_entries.read().unwrap();
        entries.get(new_name).copied()
    };
    if let Some(replaced_id) = replaced_id {
        if replaced_id == old_id {
            return 0;
        }
        let Some(replaced) = node_from_map(nodes, replaced_id) else {
            return -1;
        };
        if !can_replace_node(&node, &replaced) {
            return -1;
        }
    }

    old_entries.write().unwrap().remove(old_name);
    let mut entries = new_entries.write().unwrap();
    if let Some(replaced_id) = entries.insert(new_name.to_os_string(), old_id) {
        if let Some(replaced) = node_from_map(nodes, replaced_id) {
            *replaced.parent.write().unwrap() = None;
            *replaced.links.write().unwrap() = 0;
        }
        nodes.write().unwrap().remove(&replaced_id);
    }
    *node.parent.write().unwrap() = Some(new_parent.id);
    0
}

fn can_replace_node(source: &Node, destination: &Node) -> bool {
    match (&source.kind, &destination.kind) {
        (NodeKind::File { .. }, NodeKind::File { .. }) => true,
        (NodeKind::Directory { .. }, NodeKind::Directory { entries, .. }) => {
            entries.read().unwrap().is_empty()
        }
        _ => false,
    }
}

fn resolve_relative(start: &Arc<Node>, nodes: &NodeMap, path: &Path) -> Option<Arc<Node>> {
    if !can_resolve_relative_to_opened_node(path) {
        return None;
    }

    let mut node = start.clone();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                let NodeKind::Directory { entries, .. } = &node.kind else {
                    return None;
                };
                let child_id = *entries.read().unwrap().get(name)?;
                node = node_from_map(nodes, child_id)?;
            }
            Component::ParentDir => {
                let parent_id = node.parent.read().unwrap().as_ref().copied()?;
                node = node_from_map(nodes, parent_id)?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(node)
}

fn create_relative_node(
    start: &Arc<Node>,
    nodes: &NodeMap,
    path: &Path,
    kind: VfsEntryKind,
    mode: mode_t,
) -> Option<Arc<Node>> {
    if !can_resolve_relative_to_opened_node(path) {
        return None;
    }

    let parent_path = path.parent()?;
    let name = path.file_name()?;
    let parent = resolve_relative(start, nodes, parent_path)?;
    create_node_in_dir(&parent, nodes, name, kind, mode)
}

fn open_resolved_node(
    existing: Option<Arc<Node>>,
    create: impl FnOnce() -> Option<Arc<Node>>,
    flags: i32,
) -> (RawFd, Option<Arc<Node>>) {
    if existing.is_some() && flags & (libc::O_CREAT | libc::O_EXCL) == libc::O_CREAT | libc::O_EXCL
    {
        return (-1, None);
    }

    let node = match existing {
        Some(node) => node,
        None if flags & O_CREAT != 0 => {
            let Some(node) = create() else {
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

fn open_node_relative(
    start: &Arc<Node>,
    nodes: &NodeMap,
    path: &Path,
    flags: i32,
    mode: mode_t,
) -> Option<(RawFd, Option<Arc<Node>>)> {
    if !can_resolve_relative_to_opened_node(path) {
        return None;
    }

    let existing = resolve_relative(start, nodes, path);
    Some(open_resolved_node(
        existing,
        || create_relative_node(start, nodes, path, VfsEntryKind::File, mode),
        flags,
    ))
}

fn can_resolve_relative_to_opened_node(path: &Path) -> bool {
    !path.is_absolute()
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
    let links = *node.links.read().unwrap();
    statbuf.st_nlink = if links == 0 {
        0
    } else {
        match node.kind {
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

    fn unlink(&self, path: &Path) -> i32 {
        remove_node_at(path, &self.nodes, self.root, VfsEntryKind::File)
    }

    fn rmdir(&self, path: &Path) -> i32 {
        remove_node_at(path, &self.nodes, self.root, VfsEntryKind::Dir)
    }

    fn rename(&self, old_path: &Path, new_path: &Path) -> i32 {
        rename_node_at(old_path, new_path, &self.nodes, self.root)
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
    fn unlink_removes_file_from_namespace() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/file"), O_CREAT, 0o644) > 0);

        assert_eq!(fs.unlink(Path::new("/file")), 0);

        assert_ne!(fs.access(Path::new("/file"), F_OK), 0);
    }

    #[test]
    fn unlink_rejects_directories_and_rmdir_rejects_files() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/file"), O_CREAT, 0o644) > 0);
        assert_eq!(fs.mkdir(Path::new("/dir"), 0o755), 0);

        assert_ne!(fs.unlink(Path::new("/dir")), 0);
        assert_ne!(fs.rmdir(Path::new("/file")), 0);
        assert_eq!(fs.access(Path::new("/dir"), F_OK), 0);
        assert_eq!(fs.access(Path::new("/file"), F_OK), 0);
    }

    #[test]
    fn rmdir_only_removes_empty_directories() {
        let fs = MemoryFS::new("memory");
        assert_eq!(fs.mkdir(Path::new("/dir"), 0o755), 0);
        assert!(fs.open(Path::new("/dir/file"), O_CREAT, 0o644) > 0);

        assert_ne!(fs.rmdir(Path::new("/dir")), 0);
        assert_eq!(fs.unlink(Path::new("/dir/file")), 0);
        assert_eq!(fs.rmdir(Path::new("/dir")), 0);
        assert_ne!(fs.access(Path::new("/dir"), F_OK), 0);
    }

    #[test]
    fn rename_moves_file_entry_without_replacing_node() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/old"), O_CREAT, 0o644) > 0);
        let mut before = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/old"), &mut before), 0);

        assert_eq!(fs.rename(Path::new("/old"), Path::new("/new")), 0);

        assert_ne!(fs.access(Path::new("/old"), F_OK), 0);
        assert_eq!(fs.access(Path::new("/new"), F_OK), 0);
        let mut after = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/new"), &mut after), 0);
        assert_eq!(after.st_ino, before.st_ino);
    }

    #[test]
    fn rename_replaces_existing_file() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/old"), O_CREAT, 0o644) > 0);
        assert!(fs.open(Path::new("/new"), O_CREAT, 0o644) > 0);
        let mut before = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/old"), &mut before), 0);

        assert_eq!(fs.rename(Path::new("/old"), Path::new("/new")), 0);

        let mut after = unsafe { std::mem::zeroed() };
        assert_eq!(fs.stat(Path::new("/new"), &mut after), 0);
        assert_eq!(after.st_ino, before.st_ino);
    }

    #[test]
    fn rename_rejects_type_mismatches_and_nonempty_directory_replacement() {
        let fs = MemoryFS::new("memory");
        assert!(fs.open(Path::new("/file"), O_CREAT, 0o644) > 0);
        assert_eq!(fs.mkdir(Path::new("/dir"), 0o755), 0);
        assert_eq!(fs.mkdir(Path::new("/nonempty"), 0o755), 0);
        assert!(fs.open(Path::new("/nonempty/file"), O_CREAT, 0o644) > 0);

        assert_ne!(fs.rename(Path::new("/file"), Path::new("/dir")), 0);
        assert_ne!(fs.rename(Path::new("/dir"), Path::new("/file")), 0);
        assert_ne!(fs.rename(Path::new("/dir"), Path::new("/nonempty")), 0);
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
