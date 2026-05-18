//! Implementation of a low-level memory file system
//! It uses memfd_create to have real file descriptors. Among
//! others, they track creation and modification times

#![allow(unused)] // TODO: Remove this

#[cfg(not(target_os = "linux"))]
use std::sync::atomic::{AtomicI32, Ordering};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hasher},
    os::unix::{ffi::OsStrExt, io::RawFd},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

// use append_only_vec::AppendOnlyVec;

use libc::{c_void, mode_t};

use crate::filesystem::LowLevelFS;

// We need to keep track of all MemFS due to intercepting calls with virtual fd's
// pub static ALL_MEM_FS: AppendOnlyVec<&MemoryFS> = AppendOnlyVec::new();

#[derive(Debug)]
pub struct MemoryFS {
    id: String,
    base_fd: usize,
    cur_dir_fd: usize,
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
#[cfg(target_os = "linux")]
fn create_memfd(name: &[u8]) -> RawFd {
    use libc::{syscall, SYS_memfd_create};

    unsafe { syscall(SYS_memfd_create, name.as_ptr() as *const i8, 0) as RawFd }
}

#[cfg(not(target_os = "linux"))]
fn create_memfd(_name: &[u8]) -> RawFd {
    static NEXT_FD: AtomicI32 = AtomicI32::new(10_000);

    NEXT_FD.fetch_add(1, Ordering::Relaxed)
}

impl MemFsEntry {
    pub fn new(fd: RawFd, path: PathBuf, kind: FileKind) -> Self {
        Self { fd, path, kind }
    }
}

impl MemoryFS {
    pub fn new(id: impl Into<String>) -> Box<Self> {
        // let base_fd = ALL_MEM_FS.len() * super::MEMORY_FD_START;
        let memfs = Box::new(Self {
            id: id.into(),
            base_fd: 0,
            cur_dir_fd: 0,
            fs: RwLock::new(HashMap::new()),
        });
        // Hack: In order to avoid changing API everywhere to support a non-case, we keep static refs to the created mem filesystems
        // This is fine as long as we agree that filesystems are only destroyed at the end
        // ALL_MEM_FS.push(unsafe { std::mem::transmute(memfs.as_ref()) });
        memfs
    }

    // This intercepted call is unique to memory-fs
    pub fn getdents64(&self, fd: i32, dirp: *mut c_void, count: i32) -> isize {
        todo!()
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

    fn open(&self, path: &std::path::Path, _oflag: i32, _mode: mode_t) -> RawFd {
        if let Some(memfile) = self.fs.read().unwrap().get(path) {
            return memfile.fd;
        }
        -1
    }

    // Function to create a directory (mkdir)
    fn mkdir(&self, path: &Path, _mode: mode_t) -> RawFd {
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

    fn chmod(&self, _path: &Path, _mode: mode_t) -> i32 {
        return 0; // success
    }

    fn openat(&self, dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
        todo!()
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
    use libc::F_OK;

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
