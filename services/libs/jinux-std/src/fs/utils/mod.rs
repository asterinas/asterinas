//! VFS components

pub use access_mode::AccessMode;
pub use channel::{Channel, Consumer, Producer};
pub use creation_flags::CreationFlags;
pub use dentry_cache::Dentry;
pub use dirent_visitor::DirentVisitor;
pub use direntry_vec::DirEntryVecExt;
pub use file_creation_mask::FileCreationMask;
pub use fs::{FileSystem, FsFlags, SuperBlock};
pub use inode::{Inode, InodeMode, InodeType, Metadata};
pub use io_events::IoEvents;
pub use ioctl::IoctlCmd;
pub use page_cache::PageCache;
pub use poll::{Pollee, Poller};
pub use status_flags::StatusFlags;
pub use vnode::{Vnode, VnodeWriter};

mod access_mode;
mod channel;
mod creation_flags;
mod dentry_cache;
mod dirent_visitor;
mod direntry_vec;
mod file_creation_mask;
mod fs;
mod inode;
mod io_events;
mod ioctl;
mod page_cache;
mod poll;
mod status_flags;
mod vnode;

#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
    Start(usize),
    End(isize),
    Current(isize),
}

/// Maximum bytes in a path
pub const PATH_MAX: usize = 4096;

/// Maximum bytes in a file name
pub const NAME_MAX: usize = 255;

/// The upper limit for resolving symbolic links
pub const SYMLINKS_MAX: usize = 40;
