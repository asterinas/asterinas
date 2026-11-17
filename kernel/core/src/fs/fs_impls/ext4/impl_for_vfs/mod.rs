// SPDX-License-Identifier: MPL-2.0

//! VFS adapters for the shared EXT2/EXT4 implementation.
//!
//! `fs` exposes [`Ext4`](super::Ext4) as a VFS filesystem; `inode` adapts
//! [`Inode`](super::Inode) operations to VFS file and inode traits.

mod fs;
mod inode;
