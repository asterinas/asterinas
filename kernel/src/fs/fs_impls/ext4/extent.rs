// SPDX-License-Identifier: MPL-2.0

//! Ext4 extent tree.
//!
//! Extents replace the traditional indirect block mapping scheme used by ext2/3.

use super::prelude::*;

/// Magic number for the extent header.
const EXTENT_MAGIC: u16 = 0xF30A;

/// The root extent header lives in the inode's i_block[] area (60 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct ExtentHeader {
    /// Magic number (0xF30A).
    pub magic: u16,
    /// Number of valid entries following the header.
    pub entries: u16,
    /// Maximum number of entries that can fit.
    pub max: u16,
    /// Depth of the tree (0 = leaf nodes).
    pub depth: u16,
    /// Generation of the tree.
    pub generation: u32,
}

impl Default for ExtentHeader {
    fn default() -> Self {
        Self {
            magic: 0,
            entries: 0,
            max: 0,
            depth: 0,
            generation: 0,
        }
    }
}

/// An extent index node — points to a child block in the extent tree.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct ExtentIndex {
    /// Logical block number covered by the child block.
    pub block: u32,
    /// Lower 32 bits of the physical block number of the child.
    pub leaf_lo: u32,
    /// Upper 16 bits of the physical block number.
    pub leaf_hi: u16,
    /// Padding.
    pub _pad: u16,
}

impl Default for ExtentIndex {
    fn default() -> Self {
        Self {
            block: 0,
            leaf_lo: 0,
            leaf_hi: 0,
            _pad: 0,
        }
    }
}

impl ExtentIndex {
    pub fn leaf_bid(&self) -> u64 {
        ((self.leaf_hi as u64) << 32) | self.leaf_lo as u64
    }
}

/// A leaf extent — maps a contiguous range of logical blocks to physical blocks.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct ExtentLeaf {
    /// First logical block number covered by this extent.
    pub block: u32,
    /// Number of blocks covered by this extent.
    pub len: u16,
    /// Upper 16 bits of the physical block number.
    pub start_hi: u16,
    /// Lower 32 bits of the physical block number.
    pub start_lo: u32,
}

impl Default for ExtentLeaf {
    fn default() -> Self {
        Self {
            block: 0,
            len: 0,
            start_hi: 0,
            start_lo: 0,
        }
    }
}

impl ExtentLeaf {
    pub fn start_bid(&self) -> u64 {
        ((self.start_hi as u64) << 32) | self.start_lo as u64
    }

    pub fn len(&self) -> u32 {
        self.len as u32
    }
}

/// An extent tree reader that traverses the extent tree to find a physical block.
pub(super) struct ExtentReader {
    pub header: ExtentHeader,
    data: [u8; 60],
}

impl ExtentReader {
    /// Reads the extent header from the raw bytes (typically from inode i_block[]).
    pub fn new(data: &[u8; 60]) -> Result<Self> {
        let header = ExtentHeader::read_from_prefix(data)
            .ok_or(Error::with_message(Errno::EINVAL, "failed to read extent header"))?;
        if header.magic != EXTENT_MAGIC {
            return_errno_with_message!(Errno::EINVAL, "bad extent magic");
        }
        Ok(Self { header, data: *data })
    }

    /// Given a logical block number, find its physical block number.
    /// Returns None if the block is not mapped (sparse/hole).
    pub fn find_block(
        &self,
        logical_block: u32,
        block_device: &dyn aster_block::BlockDevice,
        block_size: usize,
    ) -> Result<Option<u64>> {
        self.find_block_recursive(logical_block, &self.header, &self.data, block_device, block_size)
    }

    fn find_block_recursive(
        &self,
        logical_block: u32,
        header: &ExtentHeader,
        data: &[u8; 60],
        block_device: &dyn aster_block::BlockDevice,
        block_size: usize,
    ) -> Result<Option<u64>> {
        let entry_size = if header.depth == 0 {
            size_of::<ExtentLeaf>()
        } else {
            size_of::<ExtentIndex>()
        };
        let entries_start = size_of::<ExtentHeader>();

        for i in 0..header.entries as usize {
            let offset = entries_start + i * entry_size;
            if header.depth == 0 {
                let leaf = ExtentLeaf::read_from_prefix(&data[offset..])
                    .ok_or(Error::with_message(Errno::EIO, "failed to read extent leaf"))?;
                let end = leaf.block as u32 + leaf.len();
                if logical_block >= leaf.block as u32 && logical_block < end {
                    let phys_block = leaf.start_bid() + (logical_block - leaf.block as u32) as u64;
                    return Ok(Some(phys_block));
                }
            } else {
                let idx = ExtentIndex::read_from_prefix(&data[offset..])
                    .ok_or(Error::with_message(Errno::EIO, "failed to read extent index"))?;
                if logical_block >= idx.block as u32 {
                    // Read the child block
                    let mut child_buf = alloc::vec![0u8; block_size];
                    let child_bid = idx.leaf_bid();
                    block_device.read_bytes(child_bid as usize * block_size, &mut child_buf)?;

                    let mut child_data = [0u8; 60];
                    let copy_len = child_buf.len().min(60);
                    child_data[..copy_len].copy_from_slice(&child_buf[..copy_len]);

                    let child_header = ExtentHeader::read_from_prefix(&child_data)
                        .ok_or(Error::with_message(Errno::EIO, "failed to read child extent header"))?;
                    if child_header.magic != EXTENT_MAGIC {
                        return_errno_with_message!(Errno::EIO, "bad child extent magic");
                    }

                    // Create full data from child block
                    let mut full_child_data = [0u8; 60];
                    let full_copy_len = child_buf.len().min(60);
                    full_child_data[..full_copy_len].copy_from_slice(&child_buf[..full_copy_len]);

                    return self.find_block_recursive(
                        logical_block,
                        &child_header,
                        &full_child_data,
                        block_device,
                        block_size,
                    );
                }
            }
        }
        // Not found — sparse hole
        Ok(None)
    }
}
