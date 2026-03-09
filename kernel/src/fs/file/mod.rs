// SPDX-License-Identifier: MPL-2.0

//! File-level abstractions and management.

mod file_attr;
mod file_handle;
pub mod file_table;
pub mod flock;
mod inode_attr;
mod inode_handle;

pub use file_attr::{AccessMode, AtomicStatusFlags, CreationFlags, OpenArgs, StatusFlags};
pub use file_handle::{FileLike, Mappable};
pub use inode_attr::{InodeMode, InodeType, Permission};
pub(crate) use inode_attr::{chmod, mkmod, perms_to_mask, who_and_perms_to_mask, who_to_mask};
pub use inode_handle::{FileIo, InodeHandle, SeekFrom};
