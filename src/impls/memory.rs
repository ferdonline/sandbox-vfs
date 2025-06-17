//! Implementation of a low-level memory file system
//! It uses memfd_create to have real file descriptors. Among
//! others, they track creation and modification times

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hasher},
    os::unix::{ffi::OsStrExt, io::RawFd},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use libc::*;

use crate::filesystem::LowLevelFS;

#[derive(Debug)]
pub struct MemoryFS {
    id: String,
    fs: RwLock<HashMap<PathBuf, Arc<MemFsEntry>>>,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct Directory {
    entries: Vec<String>, // Entries will be filenames inside the directory
    index: usize,         // Keeps track of the current position in the directory
}

// File structure for both regular files and directories
#[allow(unused)]
#[derive(Debug, Clone)]
pub struct MemFsEntry {
    fd: RawFd,
    path: PathBuf,
    kind: FileKind,
}

#[derive(Debug, Clone, Copy)]
pub enum FileKind {
    File,
    Dir,
}

/// Created a memfd entry given it's NULL TERMINATED name
fn create_memfd(name: &[u8]) -> RawFd {
    unsafe { syscall(SYS_memfd_create, name.as_ptr() as *const i8, 0) as RawFd }
}

impl MemFsEntry {
    pub fn new(fd: RawFd, path: PathBuf, kind: FileKind) -> Self {
        Self { fd, path, kind }
    }
}

impl MemoryFS {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            fs: RwLock::new(HashMap::new()),
        }
    }
}

impl LowLevelFS for MemoryFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, path: &std::path::Path, _mode: i32) -> i32 {
        // fine if file exists
        match self.fs.read().unwrap().contains_key(path) {
            true => 0,
            false => -1,
        }
    }

    fn open(&self, path: &std::path::Path, _mode: i32) -> RawFd {
        if let Some(memfile) = self.fs.read().unwrap().get(path) {
            return memfile.fd;
        }
        -1
    }

    // Function to create a directory (mkdir)
    fn mkdir(&self, path: &Path, _mode: i32) -> RawFd {
        let parent = path.parent().unwrap();
        let parent_hash = calculate_hash_seq(parent.as_os_str().as_bytes());
        let name = format!(
            "{:x}_{}\0",
            parent_hash,
            path.file_name().unwrap().to_string_lossy()
        );
        let fd = create_memfd(name.as_bytes());
        if fd < 0 {
            return -1;
        }
        #[cfg(test)]
        println!("pid: {}, memfd: {}, name: {}", std::process::id(), fd, name);

        // Add directory entry to VFS
        let fs_entry = Arc::new(MemFsEntry::new(fd, path.into(), FileKind::Dir));
        self.fs.write().unwrap().insert(path.into(), fs_entry);
        return 0; // success
    }

    fn chmod(&self, _path: &Path, _mode: i32) -> i32 {
        return 0; // success
    }
}

fn calculate_hash_seq<T: std::hash::Hash>(t: &[T]) -> u64 {
    let mut s = DefaultHasher::new();
    for e in t {
        e.hash(&mut s);
    }
    s.finish()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sys_call() {
        let fd = create_memfd("my_file\0".as_bytes());
        if fd < 0 {
            eprintln!("Err: {}", std::io::Error::last_os_error());
        }
        assert!(fd > 0);
    }

    #[test]
    fn test_fs_access() {
        let test_path = Path::new("/usr/bin/cd");
        let fs = MemoryFS::new("");

        // Not alright
        assert_ne!(fs.access(test_path, F_OK), 0);

        // .access alright after creating it
        assert_eq!(fs.mkdir(test_path, 0), 0);
        println!("fs contains: {:?}", fs);

        assert_eq!(fs.access(test_path, F_OK), 0);
    }
}
