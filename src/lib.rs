//! Virtual file system abstraction
//!
//! The virtual file system abstraction generalizes over file systems and allow using
//! different VirtualFileSystem implementations (i.e. an in memory implementation for unit tests)
//!
//! The main interaction with the virtual filesystem is by using virtual paths ([`VfsPath`](path/struct.VfsPath.html)).
//!
//! This crate currently has the following implementations:

pub mod filesystem;
pub mod impls;
pub mod libc_hooks;
pub mod root_vfs;

pub mod dlhooks;

pub use impls::bindfs::BindFS;
pub use impls::memory::MemoryFS;
pub use impls::overlay::OverlayFS;
