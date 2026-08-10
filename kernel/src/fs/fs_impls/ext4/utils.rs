// SPDX-License-Identifier: MPL-2.0

//! Small helpers shared across the ext4 module.
//!
//! - `Dirty` — a wrapper that tracks whether its inner value has been mutated
//!   since construction; it warns if dropped while dirty. Retained from the
//!   writable design — nothing is flushed on a read-only mount.
//! - `IsPowerOf` — a trait for testing whether a number is a power of another;
//!   currently unused (kept for sparse-superblock backup-group detection —
//!   backups live in groups that are powers of 3/5/7 — though the block-side
//!   lazy-group reconstruction derives its overhead from the descriptor's free
//!   count instead).
//! - `read_metadata_block` — the single funnel for metadata-block reads.

use core::ops::MulAssign;

use super::prelude::*;
use crate::prelude::warn;

#[expect(dead_code)] // Currently unused; kept for sparse-superblock backup-group detection.
pub(super) trait IsPowerOf: Copy + Sized + MulAssign + PartialOrd {
    /// Returns whether `self` equals `x^k` for some `k > 0`.
    ///
    /// `x` must be greater than 1.
    fn is_power_of(&self, x: Self) -> bool {
        let mut power = x;
        while power < *self {
            power *= x;
        }

        power == *self
    }
}

macro_rules! impl_ipo_for {
    ($($ipo_ty:ty),*) => {
        $(impl IsPowerOf for $ipo_ty {})*
    };
}

impl_ipo_for!(
    u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, isize, usize
);

/// A value with dirty tracking.
pub(super) struct Dirty<T: Debug> {
    value: T,
    dirty: bool,
}

impl<T: Debug> Dirty<T> {
    /// Creates a new `Dirty` value without setting the dirty flag.
    pub(super) fn new(val: T) -> Dirty<T> {
        Dirty {
            value: val,
            dirty: false,
        }
    }

    /// Returns whether the value is dirty.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl<T: Debug> Deref for Dirty<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Debug> DerefMut for Dirty<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.value
    }
}

impl<T: Debug> Drop for Dirty<T> {
    fn drop(&mut self) {
        if self.is_dirty() {
            warn!("dropped while dirty: {:?}", self.value);
        }
    }
}

impl<T: Debug> Debug for Dirty<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let tag = if self.dirty { "Dirty" } else { "Clean" };
        write!(f, "[{}] {:?}", tag, self.value)
    }
}

/// Reads a metadata block from the device.
///
/// All ext4 metadata-block reads (extent-tree interior/leaf nodes, inode-table
/// blocks) go through this single funnel. On a read-only, non-journaled mount
/// the on-disk block is authoritative — a clean, fully-checkpointed volume is
/// required at mount time — so this is a plain device read. Routing every
/// reader through one entry point is the seam at which a journaling layer would
/// consult its in-memory after-images (the running/committing transaction's
/// captures and any not-yet-checkpointed image) before falling through to the
/// device, without having to hunt down scattered device reads.
pub(super) fn read_metadata_block(
    device: &dyn BlockDevice,
    blocknr: Ext4Bid,
) -> Result<[u8; BLOCK_SIZE]> {
    Ok(device.read_val(Bid::new(blocknr).to_offset())?)
}
