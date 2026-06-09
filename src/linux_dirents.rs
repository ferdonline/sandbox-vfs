//! Helpers for writing Linux directory entry syscall buffers.

use std::{
    ffi::OsString,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

use libc::c_void;

use crate::filesystem::{VfsDirEntry, VfsEntryKind};

/// Result from writing directory entries into a caller-provided buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteDirentsResult {
    pub bytes_written: isize,
    pub entries_consumed: usize,
}

/// Create the synthetic `.` and `..` entries expected by directory readers.
pub fn dot_entries() -> [VfsDirEntry; 2] {
    [
        VfsDirEntry {
            name: OsString::from("."),
            kind: VfsEntryKind::Dir,
            ino: None,
        },
        VfsDirEntry {
            name: OsString::from(".."),
            kind: VfsEntryKind::Dir,
            ino: None,
        },
    ]
}

/// Write Linux `struct linux_dirent64` records into a caller-provided buffer.
///
/// The record layout is:
/// `u64 d_ino`, `i64 d_off`, `u16 d_reclen`, `u8 d_type`, then a nul-terminated
/// filename padded to 8-byte alignment. The returned value includes the number
/// of bytes written and the number of entries consumed, so callers can advance
/// their per-fd directory offset.
///
/// # Safety
///
/// `dirp` must be valid for writes of `count` bytes.
pub unsafe fn write_dirents64(
    parent_path: &Path,
    entries: &[VfsDirEntry],
    start_offset: usize,
    dirp: *mut c_void,
    count: i32,
) -> WriteDirentsResult {
    if dirp.is_null() || count <= 0 {
        return WriteDirentsResult {
            bytes_written: -1,
            entries_consumed: 0,
        };
    }

    let buf = dirp.cast::<u8>();
    let count = count as usize;
    let mut written = 0;
    let mut consumed = 0;

    for entry in entries {
        let name = entry.name.as_bytes();
        let reclen = align_to(19 + name.len() + 1, 8);
        if written + reclen > count {
            break;
        }

        let entry_path = entry_path(parent_path, &entry.name);
        let ino = entry.ino.unwrap_or_else(|| inode_for_path(&entry_path));
        let next_offset = (start_offset + consumed + 1) as i64;

        unsafe {
            ptr::write_unaligned(buf.add(written).cast::<u64>(), ino);
            ptr::write_unaligned(buf.add(written + 8).cast::<i64>(), next_offset);
            ptr::write_unaligned(buf.add(written + 16).cast::<u16>(), reclen as u16);
            ptr::write(buf.add(written + 18), dirent_type(entry.kind));
            ptr::copy_nonoverlapping(name.as_ptr(), buf.add(written + 19), name.len());
            ptr::write(buf.add(written + 19 + name.len()), 0);
            ptr::write_bytes(
                buf.add(written + 19 + name.len() + 1),
                0,
                reclen - (19 + name.len() + 1),
            );
        }

        written += reclen;
        consumed += 1;
    }

    if consumed == 0 && !entries.is_empty() {
        return WriteDirentsResult {
            bytes_written: -1,
            entries_consumed: 0,
        };
    }

    WriteDirentsResult {
        bytes_written: written as isize,
        entries_consumed: consumed,
    }
}

fn entry_path(parent_path: &Path, name: &std::ffi::OsStr) -> PathBuf {
    match name.as_bytes() {
        b"." => parent_path.to_path_buf(),
        b".." => parent_path.parent().unwrap_or(Path::new("/")).to_path_buf(),
        _ => parent_path.join(name),
    }
}

/// Round `value` up to the next multiple of `alignment`.
fn align_to(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}

/// Generate a stable-enough inode value for synthetic directory entries.
///
/// These inode numbers only need to be consistent within this process; they are
/// not host filesystem inode numbers.
fn inode_for_path(path: &Path) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Convert a VFS entry kind into the Linux `d_type` byte.
fn dirent_type(kind: VfsEntryKind) -> u8 {
    match kind {
        VfsEntryKind::File => libc::DT_REG,
        VfsEntryKind::Dir => libc::DT_DIR,
    }
}
