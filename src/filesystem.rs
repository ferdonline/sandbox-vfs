//! The filesystem trait definitions needed to implement new virtual filesystems
use std::ffi::OsString;
use std::fmt::Debug;
use std::path::Path;

use libc::{mode_t, stat};

/// Coarse file type exposed by VFS directory listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsEntryKind {
    File,
    Dir,
}

/// A single child returned from a virtual directory listing.
///
/// `name` is only the basename of the child. It must not include `/`, `.`, or
/// `..`; callers that need those synthetic entries add them at the VFS boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsDirEntry {
    pub name: OsString,
    pub kind: VfsEntryKind,
}

/// File system implementations must implement this trait
/// All path parameters are absolute, starting with '/'
pub trait LowLevelFS: Debug + Sync + Send + 'static {
    fn id(&self) -> &str; // An identifier to uniquely define the FS
    fn access(&self, path: &Path, mode: i32) -> i32;
    fn open(&self, path: &Path, flag: i32, mode: mode_t) -> i32;
    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32;
    fn mkdir(&self, path: &Path, mode: mode_t) -> i32;
    fn chmod(&self, path: &Path, mode: mode_t) -> i32;
    fn stat(&self, path: &Path, statbuf: &mut stat) -> i32;

    /// Return the direct children of `path` when the backend can enumerate it virtually.
    ///
    /// Backends that delegate directory fds to the host kernel can leave this as
    /// `None`; the libc hook will fall back to the real `getdents64` syscall.
    fn read_dir(&self, _path: &Path) -> Option<Vec<VfsDirEntry>> {
        None
    }
}
