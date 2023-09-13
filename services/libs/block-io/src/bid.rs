use crate::prelude::*;

use core::ops::{Add, Sub};

pub const BLOCK_SIZE: usize = 4096;
pub const BLOCK_BITS: usize = BLOCK_SIZE * 8;

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockId(u32);

impl BlockId {
    pub const fn new(raw_id: u32) -> Self {
        Self(raw_id)
    }

    pub const fn from_offset(offset: usize) -> Self {
        Self((offset / BLOCK_SIZE) as _)
    }

    pub fn to_offset(self) -> usize {
        (self.0 as usize) * BLOCK_SIZE
    }

    pub fn to_raw(self) -> u32 {
        self.0
    }

    pub fn offset_in_block(offset: usize) -> usize {
        offset % BLOCK_SIZE
    }
}

impl Add<u32> for BlockId {
    type Output = Self;

    fn add(self, other: u32) -> Self::Output {
        Self(self.0 + other)
    }
}

impl Sub<u32> for BlockId {
    type Output = Self;

    fn sub(self, other: u32) -> Self::Output {
        Self(self.0 - other)
    }
}

impl From<BlockId> for u32 {
    fn from(val: BlockId) -> Self {
        val.to_raw()
    }
}
