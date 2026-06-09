// SPDX-License-Identifier: MPL-2.0

//! The hidden root filesystem of the initial mount namespace.
//!
//! The mutable bootstrap rootfs is mounted over this empty filesystem. Keeping the nullfs mount
//! as the mount-tree root gives the visible rootfs a parent mount, allowing it to be replaced by a
//! later rootfs pivot.
//!
//! Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/namespace.c#L6139-L6201>.

use spin::Once;

use super::NaivePseudoFs;
use crate::prelude::*;

/// The empty filesystem used as the root mount of the initial mount namespace.
pub(in crate::fs) struct NullFs;

impl NullFs {
    /// Returns the singleton null filesystem instance.
    pub(in crate::fs) fn singleton() -> &'static Arc<NaivePseudoFs> {
        static NULLFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&NULLFS, "nullfs", NULLFS_MAGIC)
    }
}

// Nullfs is hidden from the userspace filesystem ABI and needs no public magic number.
const NULLFS_MAGIC: u64 = 0;
