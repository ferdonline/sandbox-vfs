//! The RootVFS filesystem is the one exposed to the user and the only
//! able to do mounts. A mount under a mount is still managed here!

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::RwLock,
};

use libc::{mode_t, AT_FDCWD};

use crate::filesystem::LowLevelFS;

#[derive(Debug)]
pub struct RootVFS {
    root: Box<dyn LowLevelFS>,
    mounts: HashMap<PathBuf, Box<dyn LowLevelFS>>,
    cwd: RwLock<PathBuf>,
    fds: RwLock<HashMap<i32, PathBuf>>,
}

#[allow(unused)]
impl RootVFS {
    // The root file system must be able to translate paths into the underlying fs

    pub fn new(root: Box<dyn LowLevelFS>) -> Self {
        Self {
            root,
            mounts: HashMap::new(),
            cwd: RwLock::new(PathBuf::from("/")),
            fds: RwLock::new(HashMap::new()),
        }
    }

    // For now use well defined mount points, without trailing slash
    pub fn with_mount(mut self, mount_p: impl Into<PathBuf>, fs: Box<dyn LowLevelFS>) -> Self {
        self.mounts.insert(mount_p.into(), fs);
        self
    }

    pub fn set_cwd(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        assert!(
            path.is_absolute(),
            "RootVFS cwd must be absolute, got {path:?}"
        );
        *self.cwd.write().unwrap() = Self::normalize_absolute(path);
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return Self::normalize_absolute(path);
        }

        let cwd = self.cwd.read().unwrap();
        Self::normalize_absolute(&cwd.join(path))
    }

    fn resolve_openat_path(&self, dirfd: i32, path: &Path) -> Option<PathBuf> {
        if path.is_absolute() {
            return Some(Self::normalize_absolute(path));
        }

        if dirfd == AT_FDCWD {
            return Some(self.resolve_path(path));
        }

        let fds = self.fds.read().unwrap();
        fds.get(&dirfd)
            .map(|base| Self::normalize_absolute(&base.join(path)))
    }

    fn normalize_absolute(path: &Path) -> PathBuf {
        debug_assert!(path.is_absolute());

        let mut normalized = PathBuf::from("/");
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::Normal(part) => normalized.push(part),
                Component::Prefix(_) => unreachable!("Unix paths do not have prefixes"),
            }
        }
        normalized
    }

    /// Find the actual filesystem and respective underlying path
    pub(self) fn path_to_fs(&self, in_path: &Path) -> (&dyn LowLevelFS, PathBuf) {
        let in_path = self.resolve_path(in_path);
        self.absolute_path_to_fs(in_path)
    }

    pub(self) fn absolute_path_to_fs(&self, in_path: PathBuf) -> (&dyn LowLevelFS, PathBuf) {
        debug_assert!(in_path.is_absolute());
        for p in in_path.ancestors() {
            if p == Path::new("/") {
                continue;
            }
            if let Some(mount) = self.mounts.get(p) {
                let subpath = Path::new("/").join(in_path.strip_prefix(p).unwrap());
                #[cfg(test)]
                println!("Found mount point for {:?} -> {:?}", in_path, subpath);
                return (mount.as_ref(), subpath);
            }
        }
        (self.root.as_ref(), in_path)
    }

    fn track_fd(&self, fd: i32, path: PathBuf) -> i32 {
        if fd >= 0 {
            self.fds.write().unwrap().insert(fd, path);
        }
        fd
    }

    pub fn forget_fd(&self, fd: i32) {
        self.fds.write().unwrap().remove(&fd);
    }
}

impl LowLevelFS for RootVFS {
    fn id(&self) -> &str {
        "<RootVFS>"
    }

    fn access(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.access(&path, mode)
    }

    fn open(&self, path: &Path, oflag: i32, mode: mode_t) -> i32 {
        let virtual_path = self.resolve_path(path);
        let (fs, backend_path) = self.absolute_path_to_fs(virtual_path.clone());
        self.track_fd(fs.open(&backend_path, oflag, mode), virtual_path)
    }

    fn mkdir(&self, path: &Path, mode: mode_t) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.mkdir(&path, mode)
    }

    fn chmod(&self, path: &Path, mode: mode_t) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.chmod(&path, mode)
    }

    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
        let Some(virtual_path) = self.resolve_openat_path(dirfd, path) else {
            return -1;
        };

        let (fs, backend_path) = self.absolute_path_to_fs(virtual_path.clone());
        self.track_fd(fs.open(&backend_path, flag, mode), virtual_path)
    }
}

#[cfg(test)]
mod test {

    use libc::{F_OK, O_CREAT, O_RDONLY};

    use crate::MemoryFS;

    use super::*;

    #[test]
    fn test_mount_redirect() {
        let root = RootVFS::new(MemoryFS::new("root")).with_mount("/mnt", MemoryFS::new("mnt"));

        let (fs1, p1) = root.path_to_fs(Path::new("/usr/bin"));
        assert_eq!(fs1.id(), "root");
        assert_eq!(&p1, Path::new("/usr/bin"));

        let (fs2, p2) = root.path_to_fs(Path::new("/mnt/file.txt"));
        assert_eq!(fs2.id(), "mnt");
        assert_eq!(&p2, Path::new("/file.txt"));
    }

    #[test]
    fn test_relative_paths_resolve_from_root_cwd() {
        let root = RootVFS::new(MemoryFS::new("root"));

        assert_eq!(root.mkdir(Path::new("tmp"), 0o755), 0);

        assert_eq!(root.access(Path::new("/tmp"), F_OK), 0);
    }

    #[test]
    fn test_relative_paths_resolve_from_configured_cwd() {
        let root = RootVFS::new(MemoryFS::new("root"));
        root.mkdir(Path::new("/home"), 0o755);
        root.mkdir(Path::new("/home/leite"), 0o755);
        root.set_cwd("/home/leite");

        assert!(root.open(Path::new("hello.txt"), O_CREAT, 0o644) > 0);

        assert_eq!(root.access(Path::new("/home/leite/hello.txt"), F_OK), 0);
    }

    #[test]
    fn test_relative_paths_are_normalized_before_mount_dispatch() {
        let root = RootVFS::new(MemoryFS::new("root")).with_mount("/mnt", MemoryFS::new("mnt"));
        root.set_cwd("/mnt/projects");

        let (fs, path) = root.path_to_fs(Path::new("../file.txt"));

        assert_eq!(fs.id(), "mnt");
        assert_eq!(path, Path::new("/file.txt"));
    }

    #[test]
    fn test_openat_resolves_relative_at_fdcwd() {
        let root = RootVFS::new(MemoryFS::new("root"));
        root.mkdir(Path::new("/work"), 0o755);
        root.set_cwd("/work");

        assert!(root.openat(libc::AT_FDCWD, Path::new("out.txt"), O_CREAT, 0o644) > 0);

        assert_eq!(root.access(Path::new("/work/out.txt"), F_OK), 0);
    }

    #[test]
    fn test_openat_rejects_relative_non_cwd_dirfd() {
        let root = RootVFS::new(MemoryFS::new("root"));

        assert_ne!(root.openat(123, Path::new("out.txt"), O_CREAT, 0o644), 0);
    }

    #[test]
    fn test_openat_resolves_relative_to_tracked_fd() {
        let root = RootVFS::new(MemoryFS::new("root"));
        root.mkdir(Path::new("/work"), 0o755);

        let dirfd = root.open(Path::new("/work"), O_RDONLY, 0);

        assert!(dirfd > 0);
        assert!(root.openat(dirfd, Path::new("out.txt"), O_CREAT, 0o644) > 0);
        assert_eq!(root.access(Path::new("/work/out.txt"), F_OK), 0);
    }

    #[test]
    fn test_openat_relative_to_tracked_fd_normalizes_path() {
        let root = RootVFS::new(MemoryFS::new("root"));
        root.mkdir(Path::new("/work"), 0o755);
        root.mkdir(Path::new("/work/build"), 0o755);

        let dirfd = root.open(Path::new("/work/build"), O_RDONLY, 0);

        assert!(dirfd > 0);
        assert!(root.openat(dirfd, Path::new("../out.txt"), O_CREAT, 0o644) > 0);
        assert_eq!(root.access(Path::new("/work/out.txt"), F_OK), 0);
    }

    #[test]
    fn test_openat_tracked_fd_keeps_virtual_mount_path() {
        let root = RootVFS::new(MemoryFS::new("root")).with_mount("/mnt", MemoryFS::new("mnt"));
        root.mkdir(Path::new("/mnt/work"), 0o755);

        let dirfd = root.open(Path::new("/mnt/work"), O_RDONLY, 0);

        assert!(dirfd > 0);
        assert!(root.openat(dirfd, Path::new("out.txt"), O_CREAT, 0o644) > 0);
        assert_eq!(root.access(Path::new("/mnt/work/out.txt"), F_OK), 0);
    }

    #[test]
    fn test_failed_open_is_not_tracked() {
        let root = RootVFS::new(MemoryFS::new("root"));

        let fd = root.open(Path::new("/missing"), O_RDONLY, 0);

        assert!(fd < 0);
        assert_ne!(root.openat(fd, Path::new("out.txt"), O_CREAT, 0o644), 0);
    }

    #[test]
    fn test_forget_fd_removes_openat_base() {
        let root = RootVFS::new(MemoryFS::new("root"));
        root.mkdir(Path::new("/work"), 0o755);

        let dirfd = root.open(Path::new("/work"), O_RDONLY, 0);
        root.forget_fd(dirfd);

        assert_ne!(root.openat(dirfd, Path::new("out.txt"), O_CREAT, 0o644), 0);
    }
}
