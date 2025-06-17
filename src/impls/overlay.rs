//! An overlay file system combining two filesystems, an upper layer with read/write access and a lower layer with only read access
#![allow(unused)]

use crate::filesystem::LowLevelFS;

use std::path::{Path, PathBuf};

/// An overlay file system combining several filesystems into one, an upper layer with read/write access and lower layers with only read access
///
/// Files in upper layers shadow those in lower layers. Directories are the merged view of all layers.
///
/// NOTE: To allow removing files and directories (e.g. via remove_file()) from the lower layer filesystems, this mechanism creates a `.whiteout` folder in the root of the upper level filesystem to mark removed files
///
#[derive(Debug, Clone)]
pub struct OverlayFS {
    layers: Vec<PathBuf>,
}

impl LowLevelFS for OverlayFS {
    fn access(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }

    fn open(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }
}
