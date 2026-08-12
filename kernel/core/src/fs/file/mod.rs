// SPDX-License-Identifier: MPL-2.0

//! File-level abstractions and management.

mod file_attr;
mod file_common;
mod file_handle;
pub(crate) mod file_table;
pub(crate) mod flock;
mod fs_config_file;
mod inode_attr;
mod inode_handle;

pub(crate) use file_attr::{
    access_mode::AccessMode,
    creation_flags::CreationFlags,
    open_args::OpenArgs,
    status_flags::{AtomicStatusFlags, SettableStatusFlags, StatusFlags},
};
pub(crate) use file_common::FileCommon;
pub(crate) use file_handle::{FileLike, Mappable, StatusFlagsUpdate};
pub(crate) use fs_config_file::{DetachedMountFile, FsConfigFile};
pub(crate) use inode_attr::{
    mode::{InodeMode, chmod, mkmod, perms_to_mask, who_and_perms_to_mask, who_to_mask},
    permission::Permission,
    r#type::InodeType,
};
pub(crate) use inode_handle::{InodeHandle, PerOpenFileOps, SeekFrom};
