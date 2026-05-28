//! This libc backend is for supporting inter-positioning of processes
//! It should not be used in other contexts

#![allow(unused)]

use libc::{c_char, c_int, c_void, gid_t, mode_t, size_t, uid_t};

use super::dlhooks;
use crate::filesystem::LowLevelFS;
use crate::root_vfs::RootVFS;
use crate::{libc_hooks, BindFS, FromCStr, OverlayFS};

// use crate::impls::memory::ALL_MEM_FS;

use std::env;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

const LOWER_ENV: &str = "SANDBOX_VFS_LOWER";
const UPPER_ENV: &str = "SANDBOX_VFS_UPPER";

static VFS: LazyLock<RootVFS> = LazyLock::new(|| {
    let lower = required_dir_from_env(LOWER_ENV);
    let upper = required_dir_from_env(UPPER_ENV);

    RootVFS::new(Box::new(OverlayFS::new(
        "overlay",
        Box::new(BindFS::new("upper", upper)),
        Box::new(BindFS::new("lower", lower)),
    )))
});

fn required_dir_from_env(name: &str) -> PathBuf {
    let value =
        env::var_os(name).unwrap_or_else(|| panic!("{name} must point to an existing directory"));
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|err| panic!("failed to resolve current directory for {name}: {err}"))
            .join(path)
    };

    if !path.is_dir() {
        panic!("{name} must point to an existing directory: {path:?}");
    }

    path
}

dlhooks::hook! {
    unsafe fn accessat(dirfd: c_int, cpath: *const c_char, mode: c_int, flags: c_int) -> c_int => {
        #[cfg(test)]
        libc::printf(b"> Intercepted accessat with params: %d %s %d\0".as_ptr() as *const c_char, dirfd, cpath, mode);
        Self::call_orig(dirfd, cpath, mode, flags)
    }
}
dlhooks::hook! {
    unsafe fn access(cpath: *const c_char, mode: c_int) -> c_int => {
        VFS.access(Path::from_cstr(cpath), mode)
    }
}
dlhooks::hook! {
    unsafe fn chmod(path: *const c_char, mode: mode_t) -> c_int => {
        VFS.chmod(Path::from_cstr(path), mode)
    }
}
dlhooks::hook! {
    unsafe fn fchownat(dirfd: c_int, cpath: *const c_char, uid: uid_t, gid: gid_t, flags: c_int) -> c_int => {
        #[cfg(test)]
        libc::printf(b"> Intercepted fchownat with params: %d %s %d %d %d\0".as_ptr() as *const c_char, dirfd, cpath, uid, gid, flags);
        Self::call_orig(dirfd, cpath, uid, gid, flags)
    }
}
dlhooks::hook! {
    unsafe fn lchown(path: *const c_char, uid: uid_t, gid: gid_t) -> c_int => {
        Self::call_orig(path, uid, gid)
    }
}
dlhooks::hook! {
    unsafe fn creat(path: *const c_char, mode: mode_t) -> c_int => {
        VFS.open(Path::from_cstr(path), libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC, mode)
    }
}
dlhooks::hook! {
    unsafe fn close(fd: c_int) -> c_int => {
        let result = Self::call_orig(fd);
        if result == 0 {
            VFS.forget_fd(fd);
        }
        result
    }
}
dlhooks::hook! {
    unsafe fn mkdir(path: *const c_char, mode: mode_t) -> c_int => {
        VFS.mkdir(Path::from_cstr(path), mode)
    }
}
dlhooks::hook! {
    unsafe fn mkdirat(dirfd: c_int, pathname: *const c_char, mode: mode_t) -> c_int => {
        #[cfg(test)]
        libc::printf(b"> Intercepted mkdirat with params: %d %s %d\0".as_ptr() as *const c_char, dirfd, pathname, mode as i32);
        Self::call_orig(dirfd, pathname, mode)
    }
}
dlhooks::hook! {
    unsafe fn getdents64(fd: c_int, dirp: *mut c_void, count: c_int) -> isize => {
        // We need to intercept this one operating on fd because the memory backend is not a real one
        let memfs_id = fd / 10000;
        // if memfs_id > 0 {
        //     ALL_MEM_FS[memfs_id as usize - 1].getdents64(fd, dirp, count)
        // }
        // else {
            Self::call_orig(fd, dirp, count)
        // }
    }
}

// NOTE: variadic arg functions are a bit pain since rust macros don't handle ...
// We create manually
#[no_mangle]
pub unsafe extern "C" fn open(cpath: *const c_char, oflag: c_int, mut va_args: ...) -> i32 {
    let mode = match oflag & (libc::O_CREAT | libc::O_TMPFILE) {
        0 => 0,
        _ => unsafe { va_args.arg::<libc::c_uint>() as mode_t },
    };
    VFS.open(Path::from_cstr(cpath), oflag, mode)
}

pub struct Open;
impl Open {
    pub fn call_orig(cpath: *const c_char, oflag: c_int, mode: mode_t) -> i32 {
        static REAL: OnceLock<usize> = OnceLock::new();
        let real = *REAL.get_or_init(|| crate::dlhooks::dlsym_next("open\0") as usize);
        let fn_ptr: unsafe extern "C" fn(_, _, _) -> i32 = unsafe { ::std::mem::transmute(real) };
        unsafe { fn_ptr(cpath, oflag, mode) }
    }
}

// NOTE: variadic arg functions are a bit pain since rust macros don't handle ...
// We create manually
#[no_mangle]
pub unsafe extern "C" fn openat(
    dirfd: c_int,
    cpath: *const c_char,
    oflag: c_int,
    mut va_args: ...
) -> c_int {
    let mode = match oflag & (libc::O_CREAT | libc::O_TMPFILE) {
        0 => 0,
        _ => unsafe { va_args.arg::<libc::c_uint>() as mode_t },
    };
    VFS.openat(dirfd, Path::from_cstr(cpath), oflag, mode)
}

// NOTE: variadic arg functions are a bit pain since rust macros don't handle ...
// We create manually
#[no_mangle]
pub unsafe extern "C" fn open64(cpath: *const c_char, oflag: c_int, mut va_args: ...) -> i32 {
    let mode = match oflag & (libc::O_CREAT | libc::O_TMPFILE) {
        0 => 0,
        _ => unsafe { va_args.arg::<libc::c_uint>() as mode_t },
    };
    VFS.open(Path::from_cstr(cpath), oflag, mode)
}

// dlhooks::hook! {
//     unsafe fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> c_int => {
//         Self::call_orig(path, buf, bufsiz)
//     }
// }
// dlhooks::hook! {
//     unsafe fn readlinkat(dirfd: c_int, pathname: *const c_char, buf: *mut c_char, bufsiz: size_t) -> c_int => {
//         Self::call_orig(dirfd, pathname, buf, bufsiz)
//     }
// }

// execl(path: *const c_char, arg0: *const c_char, ...) -> c_int,
// execle(path: *const c_char, arg0: *const c_char, ...) -> c_int,
// execlp(file: *const c_char, arg0: *const c_char, ...) -> c_int;
// execv(prog: *const c_char, argv: *const *mut c_char) -> c_int;
// execve(prog: *const c_char, argv: *const *mut c_char, envp: *const *mut c_char) -> c_int;
// execvp(c: *const c_char, argv: *const *mut c_char) -> c_int;

// link(src: *const c_char, dst: *const c_char) -> c_int,

// mkfifo(path: *const c_char, mode: mode_t) -> c_int,

// mknod(pathname: *const c_char, mode: mode_t, dev: crate::dev_t) -> c_int,
// mknodat(dirfd: c_int, pathname: *const c_char, mode: mode_t, dev: dev_t) -> c_int,

// pathconf(path: *const c_char, name: c_int) -> c_long,

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
