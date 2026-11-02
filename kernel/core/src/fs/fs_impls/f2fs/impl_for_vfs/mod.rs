// SPDX-License-Identifier: MPL-2.0

//! VFS trait implementations for F2FS.
//!
//! * `fs` wires [`F2fs`](super::F2fs) into the [`FileSystem`] trait.
//! * `inode` wires [`Inode`](super::Inode) into the [`FileOps`] and
//!   [`Inode`](crate::fs::vfs::inode::Inode) traits.
//!
//! Mutating operations are supported; they return `EROFS` only when the
//! volume was mounted read-only.

mod fs;
mod inode;
