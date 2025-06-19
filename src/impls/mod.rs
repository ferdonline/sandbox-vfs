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
        // Note: We use strlen + 1 to consider the final \0 char. This ensures the final path
        // can be directly "cast" to char* and used in the c routines
        let byte_len = unsafe { libc::strlen(value) } + 1;
        let bytes = unsafe { from_raw_parts(value as *const u8, byte_len) };
        let str = OsStr::from_bytes(bytes);
        Path::new(str)
    }
}

impl FromCStr for Path {}
