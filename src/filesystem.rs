//! The filesystem trait definitions needed to implement new virtual filesystems
use std::ffi::OsString;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;

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
    /// Backend-provided inode number, when the backend has stable node identity.
    pub ino: Option<u64>,
}

/// Backend-owned identity and operations for one opened filesystem object.
pub trait OpenedFile: Debug + Sync + Send + 'static {
    fn stat(&self, statbuf: &mut stat) -> i32;

    /// Open a path relative to this opened object, when the backend can do so
    /// without going back through a virtual path lookup.
    fn open_child(&self, _path: &Path, _flag: i32, _mode: mode_t) -> Option<OpenResult> {
        None
    }

    /// Create a directory relative to this opened object, when the backend can
    /// do so without going back through a virtual path lookup.
    fn mkdir_child(&self, _path: &Path, _mode: mode_t) -> Option<i32> {
        None
    }

    fn read_dir(&self) -> Option<Vec<VfsDirEntry>> {
        None
    }
}

/// A real file descriptor and, when needed, its backend-owned virtual identity.
#[derive(Debug)]
pub struct OpenResult {
    pub fd: i32,
    pub opened: Option<Arc<dyn OpenedFile>>,
}

impl OpenResult {
    pub fn from_fd(fd: i32) -> Self {
        Self { fd, opened: None }
    }
}

/// File system implementations must implement this trait
/// All path parameters are absolute, starting with '/'
pub trait LowLevelFS: Debug + Sync + Send + 'static {
    fn id(&self) -> &str; // An identifier to uniquely define the FS
    fn access(&self, path: &Path, mode: i32) -> i32;
    fn open(&self, path: &Path, flag: i32, mode: mode_t) -> i32;
    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32;
    fn mkdir(&self, path: &Path, mode: mode_t) -> i32;
    fn unlink(&self, path: &Path) -> i32;
    fn rmdir(&self, path: &Path) -> i32;
    fn rename(&self, old_path: &Path, new_path: &Path) -> i32;
    fn chmod(&self, path: &Path, mode: mode_t) -> i32;
    fn stat(&self, path: &Path, statbuf: &mut stat) -> i32;

    /// Open a path and optionally retain backend-owned identity for fd operations.
    fn open_with_handle(&self, path: &Path, flag: i32, mode: mode_t) -> OpenResult {
        OpenResult::from_fd(self.open(path, flag, mode))
    }

    /// Return the direct children of `path` when the backend can enumerate it virtually.
    ///
    /// Backends that delegate directory fds to the host kernel can leave this as
    /// `None`; the libc hook will fall back to the real `getdents64` syscall.
    fn read_dir(&self, _path: &Path) -> Option<Vec<VfsDirEntry>> {
        None
    }
}
