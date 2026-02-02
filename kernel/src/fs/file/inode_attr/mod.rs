// SPDX-License-Identifier: MPL-2.0

//! Inode attributes and persistent metadata.
//!
//! This module defines the intrinsic properties of a file system object, such
//! as its type (file, directory, etc.) and access permissions. These attributes
//! are part of the inode's persistent state and are fundamental to enforcing
//! access control and defining the object's identity within the file system.

mod mode;
mod permission;
mod r#type;

pub use mode::InodeMode;
pub(crate) use mode::{chmod, mkmod, perms_to_mask, who_and_perms_to_mask, who_to_mask};
pub use permission::Permission;
pub use r#type::InodeType;
