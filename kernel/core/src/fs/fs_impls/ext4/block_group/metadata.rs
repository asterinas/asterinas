// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use super::descriptor::{BlockGroupDesc, RawBlockGroup};
use crate::fs::{ext4::prelude::*, utils::IdBitmap};

/// Decoded allocation metadata protected by the owning block group's lock.
///
/// Each object tracks dirtiness at its own serialization granularity. Allocation
/// changes bitmap bits and descriptor counters under the same write guard.
/// Bitmap buffers are disk data; IdBitmap's search hint is never serialized.
pub(in crate::fs::fs_impls::ext4) struct BlockGroupMetadata {
    /// Group descriptor with dirty tracking.
    pub(in crate::fs::fs_impls::ext4) desc: Dirty<BlockGroupDesc>,
    /// Block bitmap cached in memory.
    pub(in crate::fs::fs_impls::ext4) block_bitmap: Dirty<IdBitmap>,
    /// Inode bitmap cached in memory.
    pub(in crate::fs::fs_impls::ext4) inode_bitmap: Dirty<IdBitmap>,
}

impl Debug for BlockGroupMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockGroupMetadata")
            .field("desc", &self.desc)
            .field("block_bitmap_dirty", &self.block_bitmap.is_dirty())
            .field("inode_bitmap_dirty", &self.inode_bitmap.is_dirty())
            .finish()
    }
}

impl BlockGroupMetadata {
    /// Writes bitmap bytes and stages a raw descriptor into the GDT buffer.
    ///
    /// The caller holds the group write lock through I/O. Successful bitmap
    /// writes clear only their own dirty state; failure leaves unfinished work
    /// retryable. A staged descriptor is clean relative to the GDT buffer, not
    /// to the device: the filesystem's dirty-group index owns GDT disk retry.
    pub(super) fn sync(
        &mut self,
        device: &dyn BlockDevice,
        group_idx: usize,
        descriptors: &USegment,
    ) -> Result<()> {
        writeback_bitmap(device, self.desc.block_bitmap_bid, &mut self.block_bitmap)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to write block bitmap"))?;
        writeback_bitmap(device, self.desc.inode_bitmap_bid, &mut self.inode_bitmap)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to write inode bitmap"))?;
        if self.desc.is_dirty() {
            let raw = RawBlockGroup::from(*self.desc);
            descriptors.write_val(group_idx * size_of::<RawBlockGroup>(), &raw)?;
            self.desc.clear_dirty();
        }
        Ok(())
    }
}

fn writeback_bitmap(
    device: &dyn BlockDevice,
    bid: Ext4Bid,
    bitmap: &mut Dirty<IdBitmap>,
) -> Result<()> {
    if !bitmap.is_dirty() {
        return Ok(());
    }
    device.write_bytes(Bid::new(u64::from(bid)).to_offset(), bitmap.as_bytes())?;
    bitmap.clear_dirty();
    Ok(())
}
