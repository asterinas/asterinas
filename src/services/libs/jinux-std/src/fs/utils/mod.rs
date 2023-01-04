//! VFS components

pub use access_mode::AccessMode;
pub use dirent_writer::{DirentWriter, DirentWriterContext};
pub use fs::{FileSystem, SuperBlock};
pub use inode::{Inode, InodeMode, InodeType, Metadata, Timespec};
pub use page_cache::PageCacheManager;
pub use status_flags::StatusFlags;

mod access_mode;
mod dirent_writer;
mod fs;
mod inode;
mod page_cache;
mod status_flags;

#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}
