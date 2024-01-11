// SPDX-License-Identifier: MPL-2.0

use super::prelude::*;

/// Represents the various ways in which a block ID can be located in Ext2.
/// It is an enum with different variants corresponding to the level of indirection
/// used to locate the block.
#[derive(Debug)]
pub enum BidPath {
    /// Direct reference to a block. The block can be accessed directly through the given
    /// index with no levels of indirection.
    Direct(usize),
    /// Single level of indirection. The block ID can be found at the specified index
    /// within an indirect block.
    Indirect(usize),
    /// Double level of indirection. The first `usize` is the index of the first-level
    /// indirect block, and the second `usize` is the index within the second-level
    /// indirect block where the block ID can be found.
    DbIndirect(usize, usize),
    /// Treble level of indirection. The three `usize` values represent the index within
    /// the first-level, second-level, and third-level indirect blocks, respectively.
    /// The block ID can be found at the third-level indirect block.
    TbIndirect(usize, usize, usize),
}

impl From<u32> for BidPath {
    fn from(bid: u32) -> Self {
        if bid < DIRECT_CNT {
            Self::Direct(bid as usize)
        } else if bid < DIRECT_CNT + INDIRECT_CNT {
            let indirect_bid = bid - DIRECT_CNT;
            Self::Indirect(indirect_bid as usize)
        } else if bid < DIRECT_CNT + INDIRECT_CNT + DB_INDIRECT_CNT {
            let db_indirect_bid = bid - (DIRECT_CNT + INDIRECT_CNT);
            let lvl1_idx = (db_indirect_bid / INDIRECT_CNT) as usize;
            let lvl2_idx = (db_indirect_bid % INDIRECT_CNT) as usize;
            Self::DbIndirect(lvl1_idx, lvl2_idx)
        } else if bid < DIRECT_CNT + INDIRECT_CNT + DB_INDIRECT_CNT + TB_INDIRECT_CNT {
            let tb_indirect_bid = bid - (DIRECT_CNT + INDIRECT_CNT + DB_INDIRECT_CNT);
            let lvl1_idx = (tb_indirect_bid / DB_INDIRECT_CNT) as usize;
            let lvl2_idx = ((tb_indirect_bid / INDIRECT_CNT) % INDIRECT_CNT) as usize;
            let lvl3_idx = (tb_indirect_bid % INDIRECT_CNT) as usize;
            Self::TbIndirect(lvl1_idx, lvl2_idx, lvl3_idx)
        } else {
            panic!("The bid: {} is too big", bid);
        }
    }
}

impl BidPath {
    /// Returns the number of blocks remaining before the next indirect block is required.
    pub fn cnt_to_next_indirect(&self) -> u32 {
        match self {
            Self::Direct(idx) => DIRECT_CNT - (*idx as u32),
            Self::Indirect(idx) | Self::DbIndirect(_, idx) | Self::TbIndirect(_, _, idx) => {
                INDIRECT_CNT - (*idx as u32)
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
            | Self::TbIndirect(_, _, idx) => *idx,
        }
    }
}

/// Direct pointers to blocks.
pub const DIRECT_RANGE: core::ops::Range<usize> = 0..12;
/// The number of direct blocks.
pub const DIRECT_CNT: u32 = DIRECT_RANGE.end as u32;

/// Indirect pointer to blocks.
pub const INDIRECT: usize = DIRECT_RANGE.end;
/// The number of indirect blocks.
pub const INDIRECT_CNT: u32 = (BLOCK_SIZE / BID_SIZE) as u32;

/// Doubly indirect pointer to blocks.
pub const DB_INDIRECT: usize = INDIRECT + 1;
/// The number of doubly indirect blocks.
pub const DB_INDIRECT_CNT: u32 = INDIRECT_CNT * INDIRECT_CNT;

/// Treble indirect pointer to blocks.
pub const TB_INDIRECT: usize = DB_INDIRECT + 1;
/// The number of trebly indirect blocks.
pub const TB_INDIRECT_CNT: u32 = INDIRECT_CNT * DB_INDIRECT_CNT;

/// The number of block pointers.
pub const BLOCK_PTR_CNT: usize = TB_INDIRECT + 1;

/// The size of of the block id.
pub const BID_SIZE: usize = core::mem::size_of::<u32>();
