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
}

#[allow(unused)]
impl RootVFS {
    // The root file system must be able to translate paths into the underlying fs

    pub fn new(root: Box<dyn LowLevelFS>) -> Self {
        Self {
            root,
            mounts: HashMap::new(),
            cwd: RwLock::new(PathBuf::from("/")),
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
        let (fs, path) = self.path_to_fs(path);
        fs.open(&path, oflag, mode)
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
        if !path.is_absolute() && dirfd != AT_FDCWD {
            return -1;
        }

        let (fs, path) = self.path_to_fs(path);
        fs.openat(dirfd, &path, flag, mode)
    }
}

#[cfg(test)]
mod test {

    use libc::{F_OK, O_CREAT};

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
}
