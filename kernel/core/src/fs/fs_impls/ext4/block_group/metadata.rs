// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(ext4)]

use core::fmt;

use super::descriptor::{BlockGroupDesc, RawBlockGroup};
use crate::fs::{ext4::prelude::*, utils::IdBitmap};

/// A block group's allocation state and pending metadata writeback.
pub(in ext4) struct BlockGroupMetadata {
    pub(in ext4) desc: Dirty<BlockGroupDesc>,
    pub(in ext4) block_bitmap: Dirty<IdBitmap>,
    pub(in ext4) inode_bitmap: Dirty<IdBitmap>,
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
    /// Successful bitmap writes become clean; failed writes remain retryable.
    /// A staged descriptor still needs its GDT buffer written to the device.
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
