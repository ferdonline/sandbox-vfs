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

use libc::{c_void, mode_t, O_CREAT};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let root = Arc::new(MemFsEntry::new(
            create_memfd(b"root\0"),
            PathBuf::from("/"),
            FileKind::Dir,
        ));
        let mut fs = HashMap::new();
        fs.insert(PathBuf::from("/"), root);

        // let base_fd = ALL_MEM_FS.len() * super::MEMORY_FD_START;
        let memfs = Box::new(Self {
            id: id.into(),
            base_fd: 0,
            cur_dir_fd: 0,
            fs: RwLock::new(fs),
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

    fn assert_absolute(path: &Path) {
        assert!(
            path.is_absolute(),
            "MemoryFS backends only accept absolute paths, got {path:?}"
        );
    }

    fn parent_is_dir(&self, path: &Path) -> bool {
        path.parent().is_some_and(|parent| {
            self.fs
                .read()
                .unwrap()
                .get(parent)
                .is_some_and(|entry| entry.kind == FileKind::Dir)
        })
    }

    fn memfd_name(path: &Path) -> Option<Vec<u8>> {
        let parent = path.parent()?;
        let file_name = path.file_name()?;
        let parent_hash = calculate_hash_seq(parent.as_os_str().as_bytes());
        Some(format!("{parent_hash:x}_{}\0", file_name.to_string_lossy()).into_bytes())
    }

    fn create_entry(&self, path: &Path, kind: FileKind) -> i32 {
        Self::assert_absolute(path);

        if path == Path::new("/") || !self.parent_is_dir(path) {
            return -1;
        }

        let mut fs = self.fs.write().unwrap();
        if fs.contains_key(path) {
            return -1;
        }

        let Some(name) = Self::memfd_name(path) else {
            return -1;
        };
        let fd = create_memfd(&name);
        if fd < 0 {
            return -1;
        }
        #[cfg(test)]
        println!(
            "pid: {}, memfd: {}, name: {}",
            std::process::id(),
            fd,
            String::from_utf8_lossy(&name)
        );

        fs.insert(
            path.into(),
            Arc::new(MemFsEntry::new(fd, path.into(), kind)),
        );
        0
    }
}

impl LowLevelFS for MemoryFS {
    fn id(&self) -> &str {
        &self.id
    }

    fn access(&self, path: &std::path::Path, _mode: i32) -> i32 {
        Self::assert_absolute(path);

        // fine if file exists
        match self.fs.read().unwrap().contains_key(path) {
            true => 0,
            false => -1,
        }
    }

    fn open(&self, path: &std::path::Path, oflag: i32, _mode: mode_t) -> RawFd {
        Self::assert_absolute(path);

        if let Some(memfile) = self.fs.read().unwrap().get(path) {
            return memfile.fd;
        }

        if oflag & O_CREAT == 0 || self.create_entry(path, FileKind::File) != 0 {
            return -1;
        }

        self.fs
            .read()
            .unwrap()
            .get(path)
            .map_or(-1, |memfile| memfile.fd)
    }

    // Function to create a directory (mkdir)
    fn mkdir(&self, path: &Path, _mode: mode_t) -> RawFd {
        self.create_entry(path, FileKind::Dir)
    }

    fn chmod(&self, path: &Path, _mode: mode_t) -> i32 {
        Self::assert_absolute(path);

        match self.fs.read().unwrap().contains_key(path) {
            true => 0,
            false => -1,
        }
    }

    fn openat(&self, _dirfd: i32, path: &Path, flag: i32, mode: mode_t) -> i32 {
        Self::assert_absolute(path);
        self.open(path, flag, mode)
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
    use libc::{F_OK, O_CREAT, O_RDONLY};

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
        assert_eq!(fs.mkdir(Path::new("/usr"), 0), 0);
        assert_eq!(fs.mkdir(Path::new("/usr/bin"), 0), 0);
        assert_eq!(fs.mkdir(test_path, 0), 0);
        println!("fs contains: {:?}", fs);

        assert_eq!(fs.access(test_path, F_OK), 0);
    }

    #[test]
    fn test_root_exists_by_default() {
        let fs = MemoryFS::new("");

        assert_eq!(fs.access(Path::new("/"), F_OK), 0);
    }

    #[test]
    fn test_mkdir_requires_existing_parent() {
        let fs = MemoryFS::new("");

        assert_ne!(fs.mkdir(Path::new("/usr/bin"), 0), 0);
        assert_ne!(fs.access(Path::new("/usr/bin"), F_OK), 0);
    }

    #[test]
    fn test_mkdir_rejects_duplicate_path() {
        let fs = MemoryFS::new("");

        assert_eq!(fs.mkdir(Path::new("/usr"), 0), 0);
        assert_ne!(fs.mkdir(Path::new("/usr"), 0), 0);
    }

    #[test]
    fn test_open_without_create_fails_for_missing_file() {
        let fs = MemoryFS::new("");

        assert_ne!(fs.open(Path::new("/hello.txt"), O_RDONLY, 0), 0);
    }

    #[test]
    fn test_open_create_creates_regular_file() {
        let fs = MemoryFS::new("");

        let fd = fs.open(Path::new("/hello.txt"), O_CREAT, 0o644);

        assert!(fd > 0);
        assert_eq!(fs.access(Path::new("/hello.txt"), F_OK), 0);
        assert_eq!(
            fs.fs
                .read()
                .unwrap()
                .get(Path::new("/hello.txt"))
                .unwrap()
                .kind,
            FileKind::File
        );
    }

    #[test]
    fn test_open_create_requires_existing_parent() {
        let fs = MemoryFS::new("");

        assert_ne!(fs.open(Path::new("/missing/hello.txt"), O_CREAT, 0o644), 0);
        assert_ne!(fs.access(Path::new("/missing/hello.txt"), F_OK), 0);
    }

    #[test]
    fn test_chmod_fails_for_missing_path() {
        let fs = MemoryFS::new("");

        assert_ne!(fs.chmod(Path::new("/missing"), 0o755), 0);
    }

    #[test]
    fn test_openat_accepts_absolute_path() {
        let fs = MemoryFS::new("");

        let fd = fs.openat(libc::AT_FDCWD, Path::new("/hello.txt"), O_CREAT, 0o644);

        assert!(fd > 0);
        assert_eq!(fs.access(Path::new("/hello.txt"), F_OK), 0);
    }

    #[test]
    #[should_panic(expected = "MemoryFS backends only accept absolute paths")]
    fn test_openat_panics_for_relative_path() {
        let fs = MemoryFS::new("");

        fs.openat(libc::AT_FDCWD, Path::new("hello.txt"), O_CREAT, 0o644);
    }

    #[test]
    #[should_panic(expected = "MemoryFS backends only accept absolute paths")]
    fn test_open_panics_for_relative_path() {
        let fs = MemoryFS::new("");

        fs.open(Path::new("hello.txt"), O_CREAT, 0o644);
    }

    #[test]
    #[should_panic(expected = "MemoryFS backends only accept absolute paths")]
    fn test_mkdir_panics_for_relative_path() {
        let fs = MemoryFS::new("");

        fs.mkdir(Path::new("tmp"), 0o755);
    }
}
