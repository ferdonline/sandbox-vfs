
//! The RootVFS filesystem is the one exposed to the user and the only
//! able to do mounts. A mount under a mount is still managed here!

use std::{collections::HashMap, path::{Path, PathBuf}};

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
        Self { root, mounts: HashMap::new() }
    }

    pub fn mount(&mut self, mount_p: impl Into<PathBuf>, fs: Box<dyn LowLevelFS>) {
        self.mounts.insert(mount_p.into(), fs);
    }

    /// Find the actual filesystem and respective underlying path
    fn path_to_fs(&self, in_path: &Path) -> (&dyn LowLevelFS, PathBuf) {
        for p in in_path.ancestors() {
            if let Some(mount) = self.mounts.get(p) {
                let subpath = Path::new("/").join(p.strip_prefix(p).unwrap());
                return (mount.as_ref(), subpath);
            }
        }
        (self.root.as_ref(), in_path.into())
    }
}

impl LowLevelFS for RootVFS {
    fn access(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.access(&path, mode)
    }

    fn open(&self, path: &Path, mode: i32) -> i32 {
        let (fs, path) = self.path_to_fs(path);
        fs.open(&path, mode)
    }
}
