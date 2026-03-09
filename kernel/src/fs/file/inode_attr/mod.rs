// SPDX-License-Identifier: MPL-2.0

//! Inode attributes and persistent metadata.
//!
//! This module defines the intrinsic properties of a file system object, such
//! as its type (file, directory, etc.) and access permissions. These attributes
//! are part of the inode's persistent state and are fundamental to enforcing
//! access control and defining the object's identity within the file system.

pub(super) mod mode;
pub(super) mod permission;
pub(super) mod r#type;
