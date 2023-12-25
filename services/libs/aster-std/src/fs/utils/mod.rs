//! VFS components

pub use access_mode::AccessMode;
pub use channel::{Channel, Consumer, Producer};
pub use creation_flags::CreationFlags;
pub use dentry::{Dentry, DentryKey};
pub use dirent_visitor::DirentVisitor;
pub use direntry_vec::DirEntryVecExt;
pub use file_creation_mask::FileCreationMask;
pub use fs::{FileSystem, FsFlags, SuperBlock};
pub use inode::{Inode, InodeMode, InodeType, Metadata};
pub use ioctl::IoctlCmd;
pub use mount::MountNode;
pub use page_cache::PageCache;
pub use status_flags::StatusFlags;

mod access_mode;
mod channel;
mod creation_flags;
mod dentry;
mod dirent_visitor;
mod direntry_vec;
mod file_creation_mask;
mod fs;
mod inode;
mod ioctl;
mod mount;
mod page_cache;
mod status_flags;

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
