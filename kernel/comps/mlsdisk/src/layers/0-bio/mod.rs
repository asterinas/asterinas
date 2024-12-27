// SPDX-License-Identifier: MPL-2.0

//! The layer of untrusted block I/O.

use static_assertions::assert_eq_size;

mod block_buf;
mod block_log;
mod block_ring;
mod block_set;

pub use self::{
    block_buf::{Buf, BufMut, BufRef},
    block_log::{BlockLog, MemLog},
    block_ring::BlockRing,
    block_set::{BlockSet, MemDisk},
};

pub type BlockId = usize;
pub const BLOCK_SIZE: usize = 0x1000;
pub const BID_SIZE: usize = core::mem::size_of::<BlockId>();

// This definition of BlockId assumes the target architecture is 64-bit
assert_eq_size!(usize, u64);
