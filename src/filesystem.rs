//! The filesystem trait definitions needed to implement new virtual filesystems

use std::fmt::Debug;
use std::path::Path;

/// File system implementations must implement this trait
/// All path parameters are absolute, starting with '/', except for the root directory
/// which is simply the empty string (i.e. "")
/// The character '/' is used to delimit directories on all platforms.
/// Path components may be any UTF-8 string, except "/", "." and ".."
///
/// Please use the test_macros [test_macros::test_vfs!] and [test_macros::test_vfs_readonly!]
pub trait LowLevelFS: Debug + Sync + Send + 'static {
    fn access(&self, path: &Path, mode: i32) -> i32;
    fn open(&self, path: &Path, mode: i32) -> i32;
}
