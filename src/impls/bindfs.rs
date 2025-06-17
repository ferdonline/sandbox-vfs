//! A file system with its root in a particular directory of another filesystem
#![allow(unused)]

use crate::{filesystem::LowLevelFS, libc_hooks};

use std::{
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

/// Similar to a chroot but done purely by path manipulation
///
/// NOTE: This mechanism should only be used for convenience, NOT FOR SECURITY
///
/// Symlinks, hardlinks, remounts, side channels and other file system mechanisms can be exploited
/// to circumvent this mechanism
#[derive(Debug, Clone)]
pub struct BindFS {
    id: String,
    root: PathBuf,
}

impl BindFS {
    /// Create a new FileSystem from the given real path
    pub fn new(id: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        BindFS {
            id: id.into(),
            root: root.into(),
        }
    }

    // Translates an absolute path within the system, to an absolute path outside it
    // Note, we should never get a relative path, the filesystem is not aware of cwd
    fn translate_path(&self, pth: impl AsRef<Path>) -> PathBuf {
        let mut components = pth.as_ref().components();
        assert_eq!(components.next().unwrap(), Component::RootDir);
        self.root.join(components.as_path())
    }
}

impl LowLevelFS for BindFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, p: &Path, mode: i32) -> i32 {
        let final_path = self.translate_path(p);
        let path_ptr = final_path.as_os_str().as_bytes().as_ptr();
        unsafe { libc_hooks::access::orig()(path_ptr as *const i8, mode) }
    }

    fn open(&self, p: &Path, mode: i32) -> i32 {
        let final_path = self.translate_path(p);
        let path_ptr = final_path.as_os_str().as_bytes().as_ptr();
        unsafe { libc_hooks::open::orig()(path_ptr as *const i8, mode) }
    }

    fn mkdir(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }

    fn chmod(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_path_translate() {
        let fs = BindFS::new("", Path::new("/opt/part1"));
        let real = fs.translate_path("/usr/bin/ls");
        assert_eq!(real, Path::new("/opt/part1/usr/bin/ls"));
    }
}
