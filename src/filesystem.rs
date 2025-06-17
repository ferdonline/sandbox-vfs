//! The filesystem trait definitions needed to implement new virtual filesystems
use std::fmt::Debug;
use std::path::Path;

/// File system implementations must implement this trait
/// All path parameters are absolute, starting with '/'
pub trait LowLevelFS: Debug + Sync + Send + 'static {
    fn id(&self) -> &str; // An identifier to uniquely define the FS
    fn access(&self, path: &Path, mode: i32) -> i32;
    fn open(&self, path: &Path, mode: i32) -> i32;
    fn mkdir(&self, path: &Path, mode: i32) -> i32;
    fn chmod(&self, path: &Path, mode: i32) -> i32;
}
