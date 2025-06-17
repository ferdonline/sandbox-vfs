//! Virtual file system abstraction
//!
//! The virtual file system abstraction generalizes over file systems and allow using
//! different VirtualFileSystem implementations (i.e. an in memory implementation for unit tests)
//!

pub mod filesystem;
pub mod impls;
pub mod root_vfs;

pub use impls::bindfs::BindFS;
pub use impls::memory::MemoryFS;
pub use impls::overlay::OverlayFS;
