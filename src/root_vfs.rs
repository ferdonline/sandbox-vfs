//! The RootVFS filesystem is the one exposed to the user and the only
//! able to do mounts. A mount under a mount is still managed here!

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::filesystem::LowLevelFS;

#[derive(Debug)]
pub struct RootVFS {
    root: Box<dyn LowLevelFS>,
    mounts: HashMap<PathBuf, Box<dyn LowLevelFS>>,
}

#[allow(unused)]
impl RootVFS {
    // The root file system must be able to translate paths into the underlying fs

    pub fn new(root: Box<dyn LowLevelFS>) -> Self {
        Self {
            root,
            mounts: HashMap::new(),
        }
    }

    // For now use well defined mount points, without trailing slash
    pub fn mount(&mut self, mount_p: impl Into<PathBuf>, fs: Box<dyn LowLevelFS>) {
        self.mounts.insert(mount_p.into(), fs);
    }

    /// Find the actual filesystem and respective underlying path
    pub(self) fn path_to_fs(&self, in_path: &Path) -> (&dyn LowLevelFS, PathBuf) {
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
        (self.root.as_ref(), in_path.into())
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

    fn open(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.open(&path, mode)
    }

    fn mkdir(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.mkdir(&path, mode)
    }

    fn chmod(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.chmod(&path, mode)
    }
}

#[cfg(test)]
mod test {

    use crate::MemoryFS;

    use super::*;

    #[test]
    fn test_mount_redirect() {
        let mut root = RootVFS::new(Box::new(MemoryFS::new("root")));
        root.mount("/mnt", Box::new(MemoryFS::new("mnt")));

        let (fs1, p1) = root.path_to_fs(Path::new("/usr/bin"));
        assert_eq!(fs1.id(), "root");
        assert_eq!(&p1, Path::new("/usr/bin"));

        let (fs2, p2) = root.path_to_fs(Path::new("/mnt/file.txt"));
        assert_eq!(fs2.id(), "mnt");
        assert_eq!(&p2, Path::new("/file.txt"));
    }
}
