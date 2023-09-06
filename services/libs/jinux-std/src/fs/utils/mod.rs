//! VFS components

pub use access_mode::AccessMode;
pub use channel::{Channel, Consumer, Producer};
pub use creation_flags::CreationFlags;
pub use dentry::{Dentry, DentryKey};
pub use file_creation_mask::FileCreationMask;
pub use mount::MountNode;
pub use page_cache::PageCache;
pub use status_flags::StatusFlags;
pub use vnode::{Vnode, VnodeWriter};

pub use jinux_vfs::device::{Device, DeviceId, DeviceType};
pub use jinux_vfs::dirent_visitor::DirentVisitor;
pub use jinux_vfs::direntry_vec::DirEntryVecExt;
pub use jinux_vfs::fs::{FileSystem, FsFlags};
pub use jinux_vfs::inode::Inode;
pub use jinux_vfs::io_events::IoEvents;
pub use jinux_vfs::ioctl::IoctlCmd;
pub use jinux_vfs::metadata::{InodeMode, InodeType, Metadata, SuperBlock};
pub use jinux_vfs::poll::{Pollee, Poller};

mod access_mode;
mod channel;
mod creation_flags;
mod dentry;
mod file_creation_mask;
mod mount;
mod page_cache;
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
