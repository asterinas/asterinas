// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use inherit_methods_macro::inherit_methods;

use super::{Buf, BufMut, BufRef};
use crate::{os::Mutex, prelude::*};

/// A log of data blocks that can support random reads and append-only
/// writes.
///
/// # Thread safety
///
/// `BlockLog` is a data structure of interior mutability.
/// It is ok to perform I/O on a `BlockLog` concurrently in multiple threads.
/// `BlockLog` promises the serialization of the append operations, i.e.,
/// concurrent appends are carried out as if they are done one by one.
pub trait BlockLog: Sync + Send {
    /// Read one or multiple blocks at a specified position.
    fn read(&self, pos: BlockId, buf: BufMut) -> Result<()>;

    /// Append one or multiple blocks at the end,
    /// returning the ID of the first newly-appended block.
    fn append(&self, buf: BufRef) -> Result<BlockId>;

    /// Ensure that blocks are persisted to the disk.
    fn flush(&self) -> Result<()>;

    /// Returns the number of blocks.
    fn nblocks(&self) -> usize;
}

macro_rules! impl_blocklog_for {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: BlockLog> BlockLog for $typ {
            fn read(&self, pos: BlockId, buf: BufMut) -> Result<()>;
            fn append(&self, buf: BufRef) -> Result<BlockId>;
            fn flush(&self) -> Result<()>;
            fn nblocks(&self) -> usize;
        }
    };
}

impl_blocklog_for!(&T, "(**self)");
impl_blocklog_for!(&mut T, "(**self)");
impl_blocklog_for!(Box<T>, "(**self)");
impl_blocklog_for!(Arc<T>, "(**self)");

/// An in-memory log that impls `BlockLog`.
pub struct MemLog {
    log: Mutex<Buf>,
    append_pos: AtomicUsize,
}

impl BlockLog for MemLog {
    fn read(&self, pos: BlockId, mut buf: BufMut) -> Result<()> {
        let nblocks = buf.nblocks();
        if pos + nblocks > self.nblocks() {
            return_errno_with_msg!(InvalidArgs, "read range out of bound");
        }
        let log = self.log.lock();
        let read_buf = &log.as_slice()[Self::offset(pos)..Self::offset(pos) + nblocks * BLOCK_SIZE];
        buf.as_mut_slice().copy_from_slice(read_buf);
        Ok(())
    }

    fn append(&self, buf: BufRef) -> Result<BlockId> {
        let nblocks = buf.nblocks();
        let mut log = self.log.lock();
        let pos = self.append_pos.load(Ordering::Acquire);
        if pos + nblocks > log.nblocks() {
            return_errno_with_msg!(InvalidArgs, "append range out of bound");
        }
        let write_buf =
            &mut log.as_mut_slice()[Self::offset(pos)..Self::offset(pos) + nblocks * BLOCK_SIZE];
        write_buf.copy_from_slice(buf.as_slice());
        self.append_pos.fetch_add(nblocks, Ordering::Release);
        Ok(pos)
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn nblocks(&self) -> usize {
        self.append_pos.load(Ordering::Acquire)
    }
}

impl MemLog {
    pub fn create(num_blocks: usize) -> Result<Self> {
        Ok(Self {
            log: Mutex::new(Buf::alloc(num_blocks)?),
            append_pos: AtomicUsize::new(0),
        })
    }

    fn offset(pos: BlockId) -> usize {
        pos * BLOCK_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_log() -> Result<()> {
        let total_blocks = 64;
        let append_nblocks = 8;
        let mem_log = MemLog::create(total_blocks)?;
        assert_eq!(mem_log.nblocks(), 0);

        let mut append_buf = Buf::alloc(append_nblocks)?;
        let content = 5_u8;
        append_buf.as_mut_slice().fill(content);
        let append_pos = mem_log.append(append_buf.as_ref())?;
        assert_eq!(append_pos, 0);
        assert_eq!(mem_log.nblocks(), append_nblocks);

        mem_log.flush()?;
        let mut read_buf = Buf::alloc(1)?;
        let read_pos = 7 as BlockId;
        mem_log.read(read_pos, read_buf.as_mut())?;
        assert_eq!(
            read_buf.as_slice(),
            &append_buf.as_slice()[read_pos * BLOCK_SIZE..(read_pos + 1) * BLOCK_SIZE]
        );
        Ok(())
    }
}
