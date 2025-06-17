//! An ephemeral in-memory file system, intended mainly for unit tests
#![allow(unused)]
use crate::filesystem::LowLevelFS;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::{Arc, RwLock};

type MemoryFsHandle = Arc<RwLock<MemoryFsImpl>>;

struct MemoryFsImpl {
    files: HashMap<String, MemoryFile>,
}

struct MemoryFile(());

/// An ephemeral in-memory file system, intended mainly for unit tests
pub struct MemoryFS {
    handle: MemoryFsHandle,
}

impl Debug for MemoryFS {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("In Memory File System")
    }
}

impl MemoryFS {
    /// Create a new in-memory filesystem
    pub fn new() -> Self {
        MemoryFS {
            handle: Arc::new(RwLock::new(MemoryFsImpl{ files: HashMap::new() })),
        }
    }
}

impl Default for MemoryFS {
    fn default() -> Self {
        Self::new()
    }
}

impl LowLevelFS for MemoryFS {
    fn access(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }

    fn open(&self, _path: &Path, _mode: i32) -> i32 {
        todo!()
    }
}
