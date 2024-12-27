// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use inherit_methods_macro::inherit_methods;

use super::{Buf, BufMut, BufRef};
use crate::{error::Errno, os::Mutex, prelude::*};

/// A fixed set of data blocks that can support random reads and writes.
///
/// # Thread safety
///
/// `BlockSet` is a data structure of interior mutability.
/// It is ok to perform I/O on a `BlockSet` concurrently in multiple threads.
/// `BlockSet` promises the atomicity of reading and writing individual blocks.
pub trait BlockSet: Sync + Send {
    /// Read one or multiple blocks at a specified position.
    fn read(&self, pos: BlockId, buf: BufMut) -> Result<()>;

    /// Read a slice of bytes at a specified byte offset.
    fn read_slice(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let start_pos = offset / BLOCK_SIZE;
        let end_pos = (offset + buf.len()).div_ceil(BLOCK_SIZE);
        if end_pos > self.nblocks() {
            return_errno_with_msg!(Errno::InvalidArgs, "read_slice position is out of range");
        }

        let nblocks = end_pos - start_pos;
        let mut blocks = Buf::alloc(nblocks)?;
        self.read(start_pos, blocks.as_mut())?;

        let offset = offset % BLOCK_SIZE;
        buf.copy_from_slice(&blocks.as_slice()[offset..offset + buf.len()]);
        Ok(())
    }

    /// Write one or multiple blocks at a specified position.
    fn write(&self, pos: BlockId, buf: BufRef) -> Result<()>;

    /// Write a slice of bytes at a specified byte offset.
    fn write_slice(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let start_pos = offset / BLOCK_SIZE;
        let end_pos = (offset + buf.len()).div_ceil(BLOCK_SIZE);
        if end_pos > self.nblocks() {
            return_errno_with_msg!(Errno::InvalidArgs, "write_slice position is out of range");
        }
        let nblocks = end_pos - start_pos;
        let mut blocks = Buf::alloc(nblocks)?;

        // Maybe we should read the first block partially.
        let start_offset = offset % BLOCK_SIZE;
        if start_offset != 0 {
            let mut start_block = Buf::alloc(1)?;
            self.read(start_pos, start_block.as_mut())?;
            blocks.as_mut_slice()[..start_offset]
                .copy_from_slice(&start_block.as_slice()[..start_offset]);
        }

        // Copy the input buffer to the write buffer.
        let end_offset = start_offset + buf.len();
        blocks.as_mut_slice()[start_offset..end_offset].copy_from_slice(buf);

        // Maybe we should read the last block partially.
        if end_offset % BLOCK_SIZE != 0 {
            let mut end_block = Buf::alloc(1)?;
            self.read(end_pos, end_block.as_mut())?;
            blocks.as_mut_slice()[end_offset..]
                .copy_from_slice(&end_block.as_slice()[end_offset % BLOCK_SIZE..]);
        }

        // Write blocks.
        self.write(start_pos, blocks.as_ref())?;
        Ok(())
    }

    /// Get a subset of the blocks in the block set.
    fn subset(&self, range: Range<BlockId>) -> Result<Self>
    where
        Self: Sized;

    /// Ensure that blocks are persisted to the disk.
    fn flush(&self) -> Result<()>;

    /// Returns the number of blocks.
    fn nblocks(&self) -> usize;
}

macro_rules! impl_blockset_for {
    ($typ:ty,$from:tt,$subset_fn:expr) => {
        #[inherit_methods(from = $from)]
        impl<T: BlockSet> BlockSet for $typ {
            fn read(&self, pos: BlockId, buf: BufMut) -> Result<()>;
            fn read_slice(&self, offset: usize, buf: &mut [u8]) -> Result<()>;
            fn write(&self, pos: BlockId, buf: BufRef) -> Result<()>;
            fn write_slice(&self, offset: usize, buf: &[u8]) -> Result<()>;
            fn flush(&self) -> Result<()>;
            fn nblocks(&self) -> usize;
            fn subset(&self, range: Range<BlockId>) -> Result<Self> {
                let closure = $subset_fn;
                closure(self, range)
            }
        }
    };
}

impl_blockset_for!(&T, "(**self)", |_this, _range| {
    return_errno_with_msg!(Errno::NotFound, "cannot return `Self` by `subset` of `&T`");
});

impl_blockset_for!(&mut T, "(**self)", |_this, _range| {
    return_errno_with_msg!(
        Errno::NotFound,
        "cannot return `Self` by `subset` of `&mut T`"
    );
});

impl_blockset_for!(Box<T>, "(**self)", |this: &T, range| {
    this.subset(range).map(|v| Box::new(v))
});

impl_blockset_for!(Arc<T>, "(**self)", |this: &Arc<T>, range| {
    (**this).subset(range).map(|v| Arc::new(v))
});

/// A disk that impl `BlockSet`.
///
/// The `region` is the accessible subset.
#[derive(Clone)]
pub struct MemDisk {
    disk: Arc<Mutex<Buf>>,
    region: Range<BlockId>,
}

impl MemDisk {
    /// Create a `MemDisk` with the number of blocks.
    pub fn create(num_blocks: usize) -> Result<Self> {
        let blocks = Buf::alloc(num_blocks)?;
        Ok(Self {
            disk: Arc::new(Mutex::new(blocks)),
            region: Range {
                start: 0,
                end: num_blocks,
            },
        })
    }
}

impl BlockSet for MemDisk {
    fn read(&self, pos: BlockId, mut buf: BufMut) -> Result<()> {
        if pos + buf.nblocks() > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "read position is out of range");
        }
        let offset = (self.region.start + pos) * BLOCK_SIZE;
        let buf_len = buf.as_slice().len();

        let disk = self.disk.lock();
        buf.as_mut_slice()
            .copy_from_slice(&disk.as_slice()[offset..offset + buf_len]);
        Ok(())
    }

    fn write(&self, pos: BlockId, buf: BufRef) -> Result<()> {
        if pos + buf.nblocks() > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "write position is out of range");
        }
        let offset = (self.region.start + pos) * BLOCK_SIZE;
        let buf_len = buf.as_slice().len();

        let mut disk = self.disk.lock();
        disk.as_mut_slice()[offset..offset + buf_len].copy_from_slice(buf.as_slice());
        Ok(())
    }

    fn subset(&self, range: Range<BlockId>) -> Result<Self> {
        if self.region.start + range.end > self.region.end {
            return_errno_with_msg!(Errno::InvalidArgs, "subset is out of range");
        }

        Ok(MemDisk {
            disk: self.disk.clone(),
            region: Range {
                start: self.region.start + range.start,
                end: self.region.start + range.end,
            },
        })
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn nblocks(&self) -> usize {
        self.region.len()
    }
}

#[cfg(test)]
mod tests {
    use core::ops::Range;

    use crate::layers::bio::{BlockSet, Buf, MemDisk};

    #[test]
    fn mem_disk() {
        let num_blocks = 64;
        let disk = MemDisk::create(num_blocks).unwrap();
        assert_eq!(disk.nblocks(), 64);

        let mut buf = Buf::alloc(1).unwrap();
        buf.as_mut_slice().fill(1);
        disk.write(32, buf.as_ref()).unwrap();

        let range = Range { start: 32, end: 64 };
        let subset = disk.subset(range).unwrap();
        assert_eq!(subset.nblocks(), 32);

        buf.as_mut_slice().fill(0);
        subset.read(0, buf.as_mut()).unwrap();
        assert_eq!(buf.as_ref().as_slice(), [1u8; 4096]);

        subset.write_slice(4096 - 4, &[2u8; 8]).unwrap();
        let mut buf = [0u8; 16];
        subset.read_slice(4096 - 8, &mut buf).unwrap();
        assert_eq!(buf, [1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0]);
    }
}
