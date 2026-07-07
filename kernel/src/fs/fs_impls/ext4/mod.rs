// SPDX-License-Identifier: MPL-2.0

//! Read-only ext4 filesystem implementation.
//!
//! A read-only ext4 built as a sibling to `ext2`, mirroring its layering and
//! visibility discipline. It mounts a clean, fully-checkpointed ext4 image and
//! serves reads: superblock parse and feature gating; extent-mapped file reads
//! (extent trees up to depth 2, Unwritten extents read as zeros); the linear
//! and htree directory read paths (`readdir`/`lookup`); and read-time
//! verification for the `metadata_csum`, `64bit`, and `flex_bg` features.
//!
//! Writing is not supported: the write-side `Inode`/`FileSystem` methods return
//! `EROFS`, and a volume that needs recovery (the `RECOVER` feature bit set, or
//! a non-empty orphan list) is refused at mount so that reads never observe a
//! crash-inconsistent on-disk state. The metadata-read funnel
//! ([`utils::read_metadata_block`]) is the seam at
//! which a journaling layer is later reintroduced.

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "ext4: "
    };
}

pub use fs::Ext4;
pub use inode::Inode;

use self::fs_type::Ext4Type;
use crate::fs::vfs::registry;

mod block_group;
mod checksum;
mod feature;
mod fs;
mod fs_type;
mod impl_for_vfs;
mod inode;
mod prelude;
mod super_block;
mod utils;

#[cfg(ktest)]
mod test_utils;

/// Registers the ext4 filesystem type with the VFS registry.
pub(super) fn init() {
    registry::register(&Ext4Type).unwrap();
}
