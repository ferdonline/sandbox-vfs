//! A file system with its root in a particular directory of another filesystem
#![allow(unused)]

use std::path::{Path, PathBuf};

/// Similar to a chroot but done purely by path manipulation
///
/// NOTE: This mechanism should only be used for convenience, NOT FOR SECURITY
///
/// Symlinks, hardlinks, remounts, side channels and other file system mechanisms can be exploited
/// to circumvent this mechanism
#[derive(Debug, Clone)]
pub struct BindFS {
    root: PathBuf,
}

impl BindFS {
    /// Create a new root FileSystem at the given virtual path
    pub fn new(root: &Path) -> Self {
        BindFS { root: root.to_path_buf() }
    }
}
