// SPDX-License-Identifier: MPL-2.0

use core::{
    iter::Step,
    ops::{Add, Sub},
};

use ostd::Pod;
use static_assertions::const_assert;

/// The block index used in the filesystem.
pub type Bid = BlockId<BLOCK_SIZE>;
/// The sector index used in the device.
pub type Sid = BlockId<SECTOR_SIZE>;

impl From<Bid> for Sid {
    fn from(bid: Bid) -> Self {
        Self::new(bid.to_raw() * (BLOCK_SIZE / SECTOR_SIZE) as u64)
    }
}

const BLOCK_SIZE: u16 = super::BLOCK_SIZE as u16;
const SECTOR_SIZE: u16 = super::SECTOR_SIZE as u16;
const_assert!(BLOCK_SIZE / SECTOR_SIZE >= 1);

/// An index of a block.
///
/// The `BlockId<const N: u16>` is a generic type that is parameterized by a constant `N`, which
/// represents the size of each block in bytes. The `BlockId<_>` provides a type-safe way of handling
/// block indices.
/// An Instance of `BlockId<_>` is guaranteed to represent valid block index, derived from byte offset
/// and the specified block size `N`.
///
/// # Examples
///
/// ```rust
/// const BLOCK_SIZE: u16 = 512;
///
/// let bytes_offset = 2048;
/// let block_id = BlockId<BLOCK_SIZE>::from_offset(bytes_offset);
/// assert!(block_id == (bytes_offset / BLOCK_SIZE));
/// ```
///
/// # Limitation
///
/// Currently, the block size is expressed in `u16`. We choose `u16` because
/// it is reasonably large to represent the common block size used in practice.
#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Pod)]
pub struct BlockId<const N: u16>(u64);

impl<const N: u16> BlockId<N> {
    /// Constructs an id from a raw id.
    pub const fn new(raw_id: u64) -> Self {
        Self(raw_id)
    }

    /// Constructs an id from a byte offset.
    pub const fn from_offset(offset: usize) -> Self {
        Self((offset / (N as usize)) as _)
    }

    /// Converts to a byte offset.
    pub fn to_offset(self) -> usize {
        (self.0 as usize) * (N as usize)
    }

    /// Converts to raw id.
    pub fn to_raw(self) -> u64 {
        self.0
    }
}

impl<const N: u16> Add<u64> for BlockId<N> {
    type Output = Self;

    fn add(self, other: u64) -> Self::Output {
        Self(self.0 + other)
    }
}

impl<const N: u16> Sub<u64> for BlockId<N> {
    type Output = Self;

    fn sub(self, other: u64) -> Self::Output {
        Self(self.0 - other)
    }
}

/// Implements the `Step` trait to iterate over `Range<Id>`.
impl<const N: u16> Step for BlockId<N> {
    fn steps_between(start: &Self, end: &Self) -> (usize, Option<usize>) {
        u64::steps_between(&start.0, &end.0)
    }

    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        u64::forward_checked(start.0, count).map(Self::new)
    }

    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        u64::backward_checked(start.0, count).map(Self::new)
    }
}
