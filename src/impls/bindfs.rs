//! A file system with its root in a particular directory of another filesystem
#![allow(unused)]

use super::AsCStr;
use libc::{c_char, mode_t};

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
        let mut pth = self.root.join(components.as_path());
        if !pth.as_os_str().as_bytes().last().is_some_and(|l| *l == 0) {
            pth.as_mut_os_string().push("\0");
        }
        pth
    }
}

impl LowLevelFS for BindFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, pth: &Path, mode: i32) -> i32 {
        let final_path = self.translate_path(pth);
        println!("  -> REAL access to {final_path:?}, o{mode:o}");
        unsafe { libc_hooks::access::call_orig(final_path.as_cstr(), mode) }
    }

    fn open(&self, pth: &Path, oflag: i32, mode: mode_t) -> i32 {
        let final_path = self.translate_path(pth);
        println!("  -> REAL open to {final_path:?}, o{mode:o}");
        unsafe { libc_hooks::Open::call_orig(final_path.as_cstr(), oflag, mode) }
    }

    fn mkdir(&self, pth: &Path, mode: mode_t) -> i32 {
        let final_path = self.translate_path(pth);
        println!("  -> REAL mkdir to {final_path:?}, o{mode:o}");
        unsafe { libc_hooks::mkdir::call_orig(final_path.as_cstr(), mode) }
    }

    fn chmod(&self, pth: &Path, mode: mode_t) -> i32 {
        let final_path = self.translate_path(pth);
        unsafe { libc_hooks::chmod::call_orig(final_path.as_cstr(), mode) }
    }

    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
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
