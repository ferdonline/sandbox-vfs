//! Virtual filesystem implementations

use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path, slice::from_raw_parts};

use libc::c_char;

pub mod bindfs;
pub mod memory;
pub mod overlay;

pub const MEMORY_FD_START: usize = 10000;

pub trait AsCStr: AsRef<OsStr> {
    /// Interprets the underlying OsStr buffer as char*, ensuring it's nul-terminated
    fn as_cstr(&self) -> *const c_char {
        let chars = self.as_ref().as_bytes();
        assert!(chars.last().is_some_and(|c| *c == 0));
        chars.as_ptr() as *const c_char
    }
}

impl AsCStr for Path {}

pub trait FromCStr {
    /// Create a Path object around the underlying given c buffer
    fn from_cstr<'a>(value: *const c_char) -> &'a Path {
        // VFS paths should not include the trailing C nul. Backends that call
        // libc add their own nul terminator at that boundary.
        let byte_len = unsafe { libc::strlen(value) };
        let bytes = unsafe { from_raw_parts(value as *const u8, byte_len) };
        let str = OsStr::from_bytes(bytes);
        Path::new(str)
    }
}

impl FromCStr for Path {}
