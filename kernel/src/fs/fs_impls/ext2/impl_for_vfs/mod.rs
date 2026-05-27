// SPDX-License-Identifier: MPL-2.0

//! VFS trait implementations for the ext2 filesystem.
//!
//! This module bridges the ext2-specific types (`Ext2`, `Inode`) to the
//! generic VFS layer.  The two child modules each contain `impl` blocks for
//! the corresponding VFS traits:
//!
//! * `fs` — `FileSystem` for `Ext2` (mount, sync, stat, root).
//! * `inode` — `InodeIo` and `Inode` for `Inode` (I/O, metadata,
//!   lookup, link, rename, symlink, extension slot, xattr).

mod fs;
mod inode;
