// SPDX-License-Identifier: MPL-2.0

//! File-level abstractions and management.

mod file_attr;
mod file_handle;
pub mod file_table;
pub mod flock;
mod inode_attr;
mod inode_handle;

pub use file_attr::{
    access_mode::AccessMode,
    creation_flags::CreationFlags,
    open_args::OpenArgs,
    status_flags::{AtomicStatusFlags, LINUX_O_LARGEFILE, StatusFlags},
};
pub use file_handle::{FileLike, Mappable, proc_fdinfo_flags, proc_fdinfo_flags_with_largefile};
pub(crate) use inode_attr::mode::{
    chmod, mkmod, perms_to_mask, who_and_perms_to_mask, who_to_mask,
};
pub use inode_attr::{mode::InodeMode, permission::Permission, r#type::InodeType};
pub use inode_handle::{InodeHandle, PerOpenFileOps, SeekFrom};
