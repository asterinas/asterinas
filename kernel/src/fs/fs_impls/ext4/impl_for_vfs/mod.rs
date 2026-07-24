// SPDX-License-Identifier: MPL-2.0

//! Wires ext4 types into the VFS trait interfaces (`FileSystem`, `FileOps`,
//! `Inode`): read entry points translate to ext4-internal operations, while the
//! write-side methods return `EROFS` on this read-only mount.

mod fs;
mod inode;
