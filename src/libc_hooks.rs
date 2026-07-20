//! This libc backend is for supporting inter-positioning of processes
//! It should not be used in other contexts

#![allow(unused)]

use libc::{c_char, c_int, c_void, gid_t, mode_t, size_t, stat as stat_t, uid_t};

use super::dlhooks;
use crate::filesystem::{LowLevelFS, VfsDirEntry, VfsEntryKind};
use crate::root_vfs::RootVFS;
use crate::{libc_hooks, BindFS, FromCStr, MemoryFS, OverlayFS};

// use crate::impls::memory::ALL_MEM_FS;

use std::collections::HashMap;
use std::env;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, OnceLock, RwLock};

const LOWER_ENV: &str = "SANDBOX_VFS_LOWER";
const UPPER_ENV: &str = "SANDBOX_VFS_UPPER";
const BACKEND_ENV: &str = "SANDBOX_VFS_BACKEND";
const MEMORY_MOUNT_ENV: &str = "SANDBOX_VFS_MEMORY_MOUNT";
const F_GETSIG_CMD: c_int = 11;
const F_SETOWN_EX_CMD: c_int = 15;
const F_GETOWN_EX_CMD: c_int = 16;
const F_GETOWNER_UIDS_CMD: c_int = 17;
const F_GET_RW_HINT_CMD: c_int = 1035;
const F_SET_RW_HINT_CMD: c_int = 1036;
const F_GET_FILE_RW_HINT_CMD: c_int = 1037;
const F_SET_FILE_RW_HINT_CMD: c_int = 1038;

#[derive(Debug)]
struct MaterializedDir {
    path: PathBuf,
    entries: Vec<VfsDirEntry>,
}

static VFS: LazyLock<RootVFS> = LazyLock::new(|| {
    match env::var(BACKEND_ENV).as_deref() {
        Ok("memory") => return RootVFS::new(MemoryFS::new("memory")),
        Ok("overlay") | Err(_) => {}
        Ok(other) => panic!("{BACKEND_ENV} must be either 'overlay' or 'memory', got {other:?}"),
    }

    let lower = required_dir_from_env(LOWER_ENV);
    let upper = required_dir_from_env(UPPER_ENV);

    let root = RootVFS::new(Box::new(OverlayFS::new(
        "overlay",
        Box::new(BindFS::new("upper", upper)),
        Box::new(BindFS::new("lower", lower)),
    )));

    match optional_absolute_path_from_env(MEMORY_MOUNT_ENV) {
        Some(path) => root.with_mount(path, MemoryFS::new("memory")),
        None => root,
    }
});
static MATERIALIZED_DIRS: LazyLock<RwLock<HashMap<usize, MaterializedDir>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static NEXT_MATERIALIZED_DIR: AtomicU64 = AtomicU64::new(0);

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

fn optional_absolute_path_from_env(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env::var_os(name)?);
    if !path.is_absolute() {
        panic!("{name} must be an absolute virtual path, got {path:?}");
    }

    Some(path)
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
    unsafe fn stat(path: *const c_char, statbuf: *mut stat_t) -> c_int => {
        VFS.stat(Path::from_cstr(path), &mut *statbuf)
    }
}
dlhooks::hook! {
    unsafe fn lstat(path: *const c_char, statbuf: *mut stat_t) -> c_int => {
        VFS.stat(Path::from_cstr(path), &mut *statbuf)
    }
}
dlhooks::hook! {
    unsafe fn fstat(fd: c_int, statbuf: *mut stat_t) -> c_int => {
        match VFS.fstat(fd, &mut *statbuf) {
            Some(result) => result,
            None => Self::call_orig(fd, statbuf),
        }
    }
}
dlhooks::hook! {
    unsafe fn fstatat(dirfd: c_int, path: *const c_char, statbuf: *mut stat_t, flags: c_int) -> c_int => {
        match VFS.resolve_at(dirfd, Path::from_cstr(path)) {
            Some(path) => VFS.stat(&path, &mut *statbuf),
            None => Self::call_orig(dirfd, path, statbuf, flags),
        }
    }
}
dlhooks::hook! {
    unsafe fn newfstatat(dirfd: c_int, path: *const c_char, statbuf: *mut stat_t, flags: c_int) -> c_int => {
        match VFS.resolve_at(dirfd, Path::from_cstr(path)) {
            Some(path) => VFS.stat(&path, &mut *statbuf),
            None => Self::call_orig(dirfd, path, statbuf, flags),
        }
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
    unsafe fn dup(fd: c_int) -> c_int => {
        let result = Self::call_orig(fd);
        if result >= 0 {
            VFS.clone_fd(fd, result);
        }
        result
    }
}
dlhooks::hook! {
    unsafe fn dup2(oldfd: c_int, newfd: c_int) -> c_int => {
        let result = Self::call_orig(oldfd, newfd);
        if result >= 0 {
            VFS.clone_fd(oldfd, result);
        }
        result
    }
}
dlhooks::hook! {
    unsafe fn dup3(oldfd: c_int, newfd: c_int, flags: c_int) -> c_int => {
        let result = Self::call_orig(oldfd, newfd, flags);
        if result >= 0 {
            VFS.clone_fd(oldfd, result);
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
        VFS.mkdirat(dirfd, Path::from_cstr(pathname), mode)
    }
}
dlhooks::hook! {
    unsafe fn unlink(path: *const c_char) -> c_int => {
        VFS.unlink(Path::from_cstr(path))
    }
}
dlhooks::hook! {
    unsafe fn rmdir(path: *const c_char) -> c_int => {
        VFS.rmdir(Path::from_cstr(path))
    }
}
dlhooks::hook! {
    unsafe fn unlinkat(dirfd: c_int, pathname: *const c_char, flags: c_int) -> c_int => {
        VFS.unlinkat(dirfd, Path::from_cstr(pathname), flags)
    }
}
dlhooks::hook! {
    unsafe fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int => {
        VFS.rename(Path::from_cstr(oldpath), Path::from_cstr(newpath))
    }
}
dlhooks::hook! {
    unsafe fn renameat(olddirfd: c_int, oldpath: *const c_char, newdirfd: c_int, newpath: *const c_char) -> c_int => {
        VFS.renameat(
            olddirfd,
            Path::from_cstr(oldpath),
            newdirfd,
            Path::from_cstr(newpath),
        )
    }
}
dlhooks::hook! {
    unsafe fn renameat2(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
        flags: libc::c_uint
    ) -> c_int => {
        if flags == 0 {
            VFS.renameat(
                olddirfd,
                Path::from_cstr(oldpath),
                newdirfd,
                Path::from_cstr(newpath),
            )
        } else {
            Self::call_orig(olddirfd, oldpath, newdirfd, newpath, flags)
        }
    }
}
dlhooks::hook! {
    unsafe fn getdents64(fd: c_int, dirp: *mut c_void, count: c_int) -> isize => {
        match VFS.getdents64(fd, dirp, count) {
            Some(result) => result,
            None => Self::call_orig(fd, dirp, count),
        }
    }
}
dlhooks::hook! {
    unsafe fn opendir(name: *const c_char) -> *mut libc::DIR => {
        let path = Path::from_cstr(name);
        if let Some(entries) = VFS.read_dir(path) {
            if let Some(materialized) = materialize_dir(&entries) {
                let c_path = path_to_cstring(&materialized.path);
                let dirp = Self::call_orig(c_path.as_ptr());
                if !dirp.is_null() {
                    MATERIALIZED_DIRS
                        .write()
                        .unwrap()
                        .insert(dirp as usize, materialized);
                    return dirp;
                }

                cleanup_materialized_dir(&materialized);
            }
        }

        Self::call_orig(name)
    }
}
dlhooks::hook! {
    unsafe fn closedir(dirp: *mut libc::DIR) -> c_int => {
        let materialized = MATERIALIZED_DIRS.write().unwrap().remove(&(dirp as usize));
        let result = Self::call_orig(dirp);
        if let Some(materialized) = materialized {
            cleanup_materialized_dir(&materialized);
        }

        result
    }
}

fn materialize_dir(entries: &[VfsDirEntry]) -> Option<MaterializedDir> {
    let path = PathBuf::from(format!(
        "/tmp/sandbox-vfs-opendir-{}-{}",
        std::process::id(),
        NEXT_MATERIALIZED_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let c_path = path_to_cstring(&path);

    if unsafe { mkdir::call_orig(c_path.as_ptr(), 0o700) } != 0 {
        return None;
    }

    let mut created = Vec::new();
    for entry in entries {
        if entry.name.as_bytes() == b"." || entry.name.as_bytes() == b".." {
            continue;
        }

        let entry_path = path.join(&entry.name);
        let c_entry_path = path_to_cstring(&entry_path);
        let result = match entry.kind {
            VfsEntryKind::Dir => unsafe { mkdir::call_orig(c_entry_path.as_ptr(), 0o700) },
            VfsEntryKind::File => unsafe {
                let fd = Open::call_orig(
                    c_entry_path.as_ptr(),
                    libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC,
                    0o600,
                );
                if fd < 0 {
                    -1
                } else {
                    close::call_orig(fd)
                }
            },
        };

        if result != 0 {
            cleanup_materialized_dir(&MaterializedDir {
                path,
                entries: created,
            });
            return None;
        }

        created.push(entry.clone());
    }

    Some(MaterializedDir {
        path,
        entries: created,
    })
}

fn cleanup_materialized_dir(materialized: &MaterializedDir) {
    for entry in materialized.entries.iter().rev() {
        let entry_path = materialized.path.join(&entry.name);
        let c_entry_path = path_to_cstring(&entry_path);
        match entry.kind {
            VfsEntryKind::Dir => unsafe {
                libc::rmdir(c_entry_path.as_ptr());
            },
            VfsEntryKind::File => unsafe {
                libc::unlink(c_entry_path.as_ptr());
            },
        }
    }

    let c_path = path_to_cstring(&materialized.path);
    unsafe {
        libc::rmdir(c_path.as_ptr());
    }
}

fn path_to_cstring(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes())
        .expect("paths with nul bytes cannot be passed to libc")
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

#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, mut va_args: ...) -> c_int {
    match cmd {
        libc::F_DUPFD | libc::F_DUPFD_CLOEXEC => {
            let minfd = unsafe { va_args.arg::<c_int>() };
            let result = Fcntl::call_orig_int(fd, cmd, minfd);
            if result >= 0 {
                VFS.clone_fd(fd, result);
            }
            result
        }
        libc::F_GETFD
        | libc::F_GETFL
        | libc::F_GETOWN
        | F_GETSIG_CMD
        | libc::F_GETLEASE
        | libc::F_GETPIPE_SZ
        | libc::F_GET_SEALS => Fcntl::call_orig_no_arg(fd, cmd),
        libc::F_GETLK
        | libc::F_SETLK
        | libc::F_SETLKW
        | libc::F_OFD_GETLK
        | libc::F_OFD_SETLK
        | libc::F_OFD_SETLKW
        | F_GETOWN_EX_CMD
        | F_SETOWN_EX_CMD
        | F_GETOWNER_UIDS_CMD
        | F_GET_RW_HINT_CMD
        | F_SET_RW_HINT_CMD
        | F_GET_FILE_RW_HINT_CMD
        | F_SET_FILE_RW_HINT_CMD => {
            let arg = unsafe { va_args.arg::<*mut c_void>() };
            Fcntl::call_orig_ptr(fd, cmd, arg)
        }
        _ => {
            let arg = unsafe { va_args.arg::<c_int>() };
            Fcntl::call_orig_int(fd, cmd, arg)
        }
    }
}

pub struct Fcntl;
impl Fcntl {
    fn orig() -> unsafe extern "C" fn(c_int, c_int, ...) -> c_int {
        static REAL: OnceLock<usize> = OnceLock::new();
        let real = *REAL.get_or_init(|| crate::dlhooks::dlsym_next("fcntl\0") as usize);
        unsafe { ::std::mem::transmute(real) }
    }

    fn call_orig_no_arg(fd: c_int, cmd: c_int) -> c_int {
        unsafe { Self::orig()(fd, cmd) }
    }

    fn call_orig_int(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
        unsafe { Self::orig()(fd, cmd, arg) }
    }

    fn call_orig_ptr(fd: c_int, cmd: c_int, arg: *mut c_void) -> c_int {
        unsafe { Self::orig()(fd, cmd, arg) }
    }
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
