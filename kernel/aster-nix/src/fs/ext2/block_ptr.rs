// SPDX-License-Identifier: MPL-2.0

use super::prelude::*;

pub type Ext2Bid = u32;

/// The pointers to blocks for an inode.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod)]
pub struct BlockPtrs {
    inner: [Ext2Bid; MAX_BLOCK_PTRS],
}

impl BlockPtrs {
    /// Returns the direct block ID.
    ///
    /// # Panics
    ///
    /// If the `idx` is out of bounds, this method will panic.
    pub fn direct(&self, idx: usize) -> Ext2Bid {
        assert!(DIRECT_RANGE.contains(&idx));
        self.inner[idx]
    }

    /// Sets the direct block ID.
    ///
    /// # Panics
    ///
    /// If the `idx` is out of bounds, this method will panic.
    pub fn set_direct(&mut self, idx: usize, bid: Ext2Bid) {
        assert!(DIRECT_RANGE.contains(&idx));
        self.inner[idx] = bid;
    }

    /// Returns the block ID of single indirect block pointer.
    pub fn indirect(&self) -> Ext2Bid {
        self.inner[INDIRECT]
    }

    /// Sets the block ID of single indirect block pointer.
    pub fn set_indirect(&mut self, bid: Ext2Bid) {
        self.inner[INDIRECT] = bid;
    }

    /// Returns the block ID of double indirect block pointer.
    pub fn db_indirect(&self) -> Ext2Bid {
        self.inner[DB_INDIRECT]
    }

    /// Sets the block ID of double indirect block pointer.
    pub fn set_db_indirect(&mut self, bid: Ext2Bid) {
        self.inner[DB_INDIRECT] = bid;
    }

    /// Returns the block ID of treble indirect block pointer.
    pub fn tb_indirect(&self) -> Ext2Bid {
        self.inner[TB_INDIRECT]
    }

    /// Sets the block ID of treble indirect block pointer.
    pub fn set_tb_indirect(&mut self, bid: Ext2Bid) {
        self.inner[TB_INDIRECT] = bid;
    }

    /// Views it as a slice of `u8` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Views it as a mutable slice of `u8` bytes.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.inner.as_bytes_mut()
    }
}

/// Represents the various ways in which a block ID can be located in Ext2.
/// It is an enum with different variants corresponding to the level of indirection
/// used to locate the block.
///
/// We choose `u16` because it is reasonably large to represent the index.
#[derive(Debug)]
pub enum BidPath {
    /// Direct reference to a block. The block can be accessed directly through the given
    /// index with no levels of indirection.
    Direct(u16),
    /// Single level of indirection. The block ID can be found at the specified index
    /// within an indirect block.
    Indirect(u16),
    /// Double level of indirection. The first item is the index of the first-level
    /// indirect block, and the second item is the index within the second-level
    /// indirect block where the block ID can be found.
    DbIndirect(u16, u16),
    /// Treble level of indirection. The three values represent the index within the
    /// first-level, second-level, and third-level indirect blocks, respectively.
    /// The block ID can be found at the third-level indirect block.
    TbIndirect(u16, u16, u16),
}

impl From<Ext2Bid> for BidPath {
    fn from(bid: Ext2Bid) -> Self {
        if bid < MAX_DIRECT_BLOCKS {
            Self::Direct(bid as u16)
        } else if bid < MAX_DIRECT_BLOCKS + MAX_INDIRECT_BLOCKS {
            let indirect_bid = bid - MAX_DIRECT_BLOCKS;
            Self::Indirect(indirect_bid as u16)
        } else if bid < MAX_DIRECT_BLOCKS + MAX_INDIRECT_BLOCKS + MAX_DB_INDIRECT_BLOCKS {
            let db_indirect_bid = bid - (MAX_DIRECT_BLOCKS + MAX_INDIRECT_BLOCKS);
            let lvl1_idx = (db_indirect_bid / MAX_INDIRECT_BLOCKS) as u16;
            let lvl2_idx = (db_indirect_bid % MAX_INDIRECT_BLOCKS) as u16;
            Self::DbIndirect(lvl1_idx, lvl2_idx)
        } else if bid
            < MAX_DIRECT_BLOCKS
                + MAX_INDIRECT_BLOCKS
                + MAX_DB_INDIRECT_BLOCKS
                + MAX_TB_INDIRECT_BLOCKS
        {
            let tb_indirect_bid =
                bid - (MAX_DIRECT_BLOCKS + MAX_INDIRECT_BLOCKS + MAX_DB_INDIRECT_BLOCKS);
            let lvl1_idx = (tb_indirect_bid / MAX_DB_INDIRECT_BLOCKS) as u16;
            let lvl2_idx = ((tb_indirect_bid / MAX_INDIRECT_BLOCKS) % MAX_INDIRECT_BLOCKS) as u16;
            let lvl3_idx = (tb_indirect_bid % MAX_INDIRECT_BLOCKS) as u16;
            Self::TbIndirect(lvl1_idx, lvl2_idx, lvl3_idx)
        } else {
            // The bid value in Ext2 must not surpass the representation of BidPath.
            unreachable!();
        }
    }
}

impl BidPath {
    /// Returns the number of blocks remaining before the next indirect block is required.
    pub fn cnt_to_next_indirect(&self) -> Ext2Bid {
        match self {
            Self::Direct(idx) => MAX_DIRECT_BLOCKS - (*idx as Ext2Bid),
            Self::Indirect(idx) | Self::DbIndirect(_, idx) | Self::TbIndirect(_, _, idx) => {
                MAX_INDIRECT_BLOCKS - (*idx as Ext2Bid)
            }
        }
    }

    /// Returns the last level index.
    ///
    /// This index corresponds to the position of a block within the most deeply nested
    /// indirect block (if any), or the direct block index if no indirection is involved.
    pub fn last_lvl_idx(&self) -> usize {
        match self {
            Self::Direct(idx)
            | Self::Indirect(idx)
            | Self::DbIndirect(_, idx)
            | Self::TbIndirect(_, _, idx) => *idx as _,
        }
    }
}

/// Direct pointers to blocks.
pub const DIRECT_RANGE: core::ops::Range<usize> = 0..12;
/// The number of direct blocks.
pub const MAX_DIRECT_BLOCKS: Ext2Bid = DIRECT_RANGE.end as Ext2Bid;

/// Indirect pointer to blocks.
pub const INDIRECT: usize = DIRECT_RANGE.end;
/// The number of indirect blocks.
pub const MAX_INDIRECT_BLOCKS: Ext2Bid = (BLOCK_SIZE / BID_SIZE) as Ext2Bid;

/// Doubly indirect pointer to blocks.
pub const DB_INDIRECT: usize = INDIRECT + 1;
/// The number of doubly indirect blocks.
pub const MAX_DB_INDIRECT_BLOCKS: Ext2Bid = MAX_INDIRECT_BLOCKS * MAX_INDIRECT_BLOCKS;

/// Treble indirect pointer to blocks.
pub const TB_INDIRECT: usize = DB_INDIRECT + 1;
/// The number of trebly indirect blocks.
pub const MAX_TB_INDIRECT_BLOCKS: Ext2Bid = MAX_INDIRECT_BLOCKS * MAX_DB_INDIRECT_BLOCKS;

/// The number of block pointers.
pub const MAX_BLOCK_PTRS: usize = TB_INDIRECT + 1;

/// The size of of the block id.
pub const BID_SIZE: usize = core::mem::size_of::<Ext2Bid>();
