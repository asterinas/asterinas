// SPDX-License-Identifier: MPL-2.0

//! Wires ext4 types into the VFS trait interfaces (`FileSystem`, `FileOps`,
//! and `Inode`). The implementations are thin adapters around ext4-internal
//! operations.

mod fs;
mod inode;
