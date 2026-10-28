// SPDX-License-Identifier: MPL-2.0

use ostd::const_assert;

use crate::fs::{ext4::prelude::*, utils::IdBitmap};

/// Decoded block-group addresses and allocation counters.
///
/// This value contains persistent metadata only. The owning block group keeps
/// its lock, bitmap caches, inode cache, and writeback scheduling separately.
#[derive(Clone, Copy, Debug)]
pub(in crate::fs::fs_impls::ext4) struct BlockGroupDesc {
    pub block_bitmap_bid: Ext4Bid,
    pub inode_bitmap_bid: Ext4Bid,
    pub inode_table_bid: Ext4Bid,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub used_dirs_count: u16,
}

impl From<RawBlockGroup> for BlockGroupDesc {
    fn from(raw: RawBlockGroup) -> Self {
        Self {
            block_bitmap_bid: raw.block_bitmap_bid,
            inode_bitmap_bid: raw.inode_bitmap_bid,
            inode_table_bid: raw.inode_table_bid,
            free_blocks_count: raw.free_blocks_count,
            free_inodes_count: raw.free_inodes_count,
            used_dirs_count: raw.used_dirs_count,
        }
    }
}

impl From<BlockGroupDesc> for RawBlockGroup {
    fn from(desc: BlockGroupDesc) -> Self {
        Self {
            block_bitmap_bid: desc.block_bitmap_bid,
            inode_bitmap_bid: desc.inode_bitmap_bid,
            inode_table_bid: desc.inode_table_bid,
            free_blocks_count: desc.free_blocks_count,
            free_inodes_count: desc.free_inodes_count,
            used_dirs_count: desc.used_dirs_count,
            pad: 0,
            reserved: [0; 3],
        }
    }
}

impl BlockGroupDesc {
    /// Validates that free counters fit within this group's capacity.
    pub(super) fn validate_free_counts(
        &self,
        nr_blocks_in_group: u32,
        nr_inodes_in_group: u32,
    ) -> Result<()> {
        if u32::from(self.free_blocks_count) > nr_blocks_in_group {
            return_errno_with_message!(
                Errno::EINVAL,
                "group free blocks count exceeds group blocks count"
            );
        }
        if u32::from(self.free_inodes_count) > nr_inodes_in_group {
            return_errno_with_message!(
                Errno::EINVAL,
                "group free inodes count exceeds group inodes count"
            );
        }

        Ok(())
    }

    /// Validates that all metadata block pointers (block bitmap, inode bitmap,
    /// inode table) fall within the group range and are marked as allocated.
    pub(super) fn validate_metadata_blocks(
        &self,
        block_bitmap: &IdBitmap,
        first_block: u32,
        last_block: u32,
        nr_inode_table_blocks_per_group: u32,
    ) -> Result<()> {
        let max_bit = last_block - first_block;

        let is_valid_fn = |bid: u32| -> bool {
            let Some(offset) = bid.checked_sub(first_block) else {
                return false;
            };
            if offset > max_bit {
                return false;
            }
            block_bitmap.is_allocated(offset as u16)
        };

        if !is_valid_fn(self.block_bitmap_bid) {
            return_errno_with_message!(Errno::EINVAL, "block bitmap block invalid or not marked");
        }
        if !is_valid_fn(self.inode_bitmap_bid) {
            return_errno_with_message!(Errno::EINVAL, "inode bitmap block invalid or not marked");
        }

        if self.inode_table_bid < first_block || self.inode_table_bid - first_block > max_bit {
            return_errno_with_message!(Errno::EINVAL, "inode table start out of group range");
        }
        let table_start_bit = self.inode_table_bid - first_block;
        let table_end_bit = table_start_bit + nr_inode_table_blocks_per_group;
        if table_end_bit - 1 > max_bit {
            return_errno_with_message!(Errno::EINVAL, "inode table extends beyond group");
        }
        for table_bit in table_start_bit..table_end_bit {
            if !block_bitmap.is_allocated(table_bit as u16) {
                return_errno_with_message!(Errno::EINVAL, "inode table block not marked in bitmap");
            }
        }

        Ok(())
    }
}

/// On-disk block group descriptor (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in crate::fs::fs_impls::ext4) struct RawBlockGroup {
    pub block_bitmap_bid: u32,  // bg_block_bitmap
    pub inode_bitmap_bid: u32,  // bg_inode_bitmap
    pub inode_table_bid: u32,   // bg_inode_table
    pub free_blocks_count: u16, // bg_free_blocks_count
    pub free_inodes_count: u16, // bg_free_inodes_count
    pub used_dirs_count: u16,   // bg_used_dirs_count
    pub pad: u16,               // bg_pad
    pub reserved: [u32; 3],     // bg_reserved
}

const_assert!(size_of::<RawBlockGroup>() == 32);
