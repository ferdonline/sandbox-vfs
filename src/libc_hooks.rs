//! This libc backend is for supporting inter-positioning of processes
//! It should not be used in other contexts

use std::ffi::{c_int, c_char, CString};

use crate::FileSystem;

#[derive(Debug)]
pub struct LibcBackend;

impl LibcBackend {}

impl FileSystem for LibcBackend {
    fn read_dir(&self, _path: &str) -> crate::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
        todo!()
    }

    fn create_dir(&self, _path: &str) -> crate::VfsResult<()> {
        todo!()
    }

    fn open_file(&self, _path: &str) -> crate::VfsResult<Box<dyn crate::SeekAndRead + Send>> {
        todo!()
    }

    fn create_file(&self, _path: &str) -> crate::VfsResult<Box<dyn crate::SeekAndWrite + Send>> {
        todo!()
    }

    fn append_file(&self, _path: &str) -> crate::VfsResult<Box<dyn crate::SeekAndWrite + Send>> {
        todo!()
    }

    fn metadata(&self, _path: &str) -> crate::VfsResult<crate::VfsMetadata> {
        todo!()
    }

    fn exists(&self, _path: &str) -> crate::VfsResult<bool> {
        todo!()
    }

    fn remove_file(&self, _path: &str) -> crate::VfsResult<()> {
        todo!()
    }

    fn remove_dir(&self, _path: &str) -> crate::VfsResult<()> {
        todo!()
    }

    fn set_creation_time(&self, _path: &str, _time: std::time::SystemTime) -> crate::VfsResult<()> {
        Err(crate::VfsError::from(crate::error::VfsErrorKind::NotSupported))
    }

    fn set_modification_time(&self, _path: &str, _time: std::time::SystemTime) -> crate::VfsResult<()> {
        Err(crate::VfsError::from(crate::error::VfsErrorKind::NotSupported))
    }

    fn set_access_time(&self, _path: &str, _time: std::time::SystemTime) -> crate::VfsResult<()> {
        Err(crate::VfsError::from(crate::error::VfsErrorKind::NotSupported))
    }

    fn copy_file(&self, _src: &str, _dest: &str) -> crate::VfsResult<()> {
        Err(crate::error::VfsErrorKind::NotSupported.into())
    }

    fn move_file(&self, _src: &str, _dest: &str) -> crate::VfsResult<()> {
        Err(crate::error::VfsErrorKind::NotSupported.into())
    }

    fn move_dir(&self, _src: &str, _dest: &str) -> crate::VfsResult<()> {
        Err(crate::error::VfsErrorKind::NotSupported.into())
    }

    fn _access(&self, path: &str, mode: i32) -> i32 {
        let c_path = CString::new(path).unwrap();
        unsafe { access.get()(c_path.as_ptr(), mode) }
    }

}

// hooks

redhook::hook!{
    unsafe fn accessat(_dirfd: c_int, _pathname: *const c_char, _mode: c_int, _flags: c_int) -> c_int => faccess_wrapper {
        42
    }
}
redhook::hook!{
    unsafe fn access(_path: *const c_char, _amode: c_int) -> c_int => access_wrapper {
        42
    }
}
redhook::hook!{
    unsafe fn open(_cpath: *const c_char, _oflag: c_int) -> c_int => open_wrapper {
        42
    }
}
redhook::hook!{
    unsafe fn openat(_dirfd: c_int, _cpath: *const c_char, _oflag: c_int) -> c_int => openat_wrapper {
        42
    }
}

// fchownat(dirfd: c_int, pathname: *const c_char, owner: crate::uid_t, group: crate::gid_t, flags: c_int) -> c_int
// lchown(path: *const c_char, uid: uid_t, gid: gid_t) -> c_int

// creat(path: *const c_char, mode: mode_t) -> c_int,

// execl(path: *const c_char, arg0: *const c_char, ...) -> c_int,
// execle(path: *const c_char, arg0: *const c_char, ...) -> c_int,
// execlp(file: *const c_char, arg0: *const c_char, ...) -> c_int;
// execv(prog: *const c_char, argv: *const *mut c_char) -> c_int;
// execve(prog: *const c_char, argv: *const *mut c_char, envp: *const *mut c_char) -> c_int;
// execvp(c: *const c_char, argv: *const *mut c_char) -> c_int;

// link(src: *const c_char, dst: *const c_char) -> c_int,

// mkdir(path: *const c_char, mode: mode_t) -> c_int,
// mkdirat(dirfd: c_int, pathname: *const c_char, mode: mode_t) -> c_int,

// mkfifo(path: *const c_char, mode: mode_t) -> c_int,

// mknod(pathname: *const c_char, mode: mode_t, dev: crate::dev_t) -> c_int,
// mknodat(dirfd: c_int, pathname: *const c_char, mode: mode_t, dev: dev_t) -> c_int,

// pathconf(path: *const c_char, name: c_int) -> c_long,

// readlink(path: *const c_char, buf: *mut c_char, bufsz: size_t) -> c_int,
// readlinkat(
//     dirfd: c_int,
//     pathname: *const c_char,
//     buf: *mut c_char,
//     bufsiz: size_t,
// ) -> c_int

// realpath(pathname: *const c_char, resolved: *mut c_char) -> *mut c_char,

// rmdir(path: *const c_char) -> c_int

// stat(path: *const c_char, buf: *mut stat) -> c_int,
// fstatat(dirfd: c_int, pathname: *const c_char, buf: *mut stat, flags: c_int) -> c_int;
// fstatat64(
//     dirfd: c_int,
//     pathname: *const c_char,
//     buf: *mut stat64,
//     flags: c_int,
// ) -> c_int,
// ???
// statx(
//     dirfd: c_int,
//     pathname: *const c_char,
//     flags: c_int,
//     mask: c_uint,
//     statxbuf: *mut statx,
// ) -> c_int,

// ???
// statvfs(path: *const c_char, buf: *mut statvfs) -> c_int

// truncate(path: *const c_char, length: off_t) -> c_int,

// unlinkat(dirfd: c_int, pathname: *const c_char, flags: c_int) -> c_int,

// utime(file: *const c_char, buf: *const utimbuf) -> c_int,

// Multiple path args

// linkat(
//     olddirfd: c_int,
//     oldpath: *const c_char,
//     newdirfd: c_int,
//     newpath: *const c_char,
//     flags: c_int
// ) -> c_int,

// renameat(
//     olddirfd: c_int,
//     oldpath: *const c_char,
//     newdirfd: c_int,
//     newpath: *const c_char
// ) -> c_int,

// renameat2(
//     olddirfd: c_int,
//     oldpath: *const c_char,
//     newdirfd: c_int,
//     newpath: *const c_char,
//     flags: c_uint
// ) -> c_int
