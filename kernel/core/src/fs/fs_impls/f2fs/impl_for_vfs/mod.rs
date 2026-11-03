// SPDX-License-Identifier: MPL-2.0

//! VFS trait implementations for F2FS.
//!
//! * `fs` wires [`F2fs`](super::fs::F2fs) into the
//!   [`FileSystem`](crate::fs::vfs::file_system::FileSystem) trait.
//! * `inode` wires [`Inode`](super::inode::Inode) into the
//!   [`FileOps`](crate::fs::vfs::inode::FileOps) and
//!   [`Inode`](crate::fs::vfs::inode::Inode) traits.
//!
//! Mutating operations are supported; they return `EROFS` only when the
//! volume was mounted read-only.

mod fs;
mod inode;
