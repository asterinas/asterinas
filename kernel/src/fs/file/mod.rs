// SPDX-License-Identifier: MPL-2.0

//! File-level abstractions and management.

mod file_attr;
mod file_handle;
pub mod file_table;
pub mod flock;
mod inode_attr;
mod inode_handle;

pub use file_attr::*;
pub use file_handle::{FileLike, Mappable, SeekFrom};
pub use inode_attr::*;
pub use inode_handle::{FileIo, InodeHandle};
