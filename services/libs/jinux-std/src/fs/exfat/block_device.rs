use jinux_frame::vm::{VmIo, VmFrame};
use crate::prelude::*;
use alloc::fmt::Debug;
/// A simple block device for Exfat.
pub trait BlockDevice: Send + Sync + Any {
    ///Returns the number of blocks.
    fn blocks_count(&self) -> usize;

    /// Reads a `[u8]` slice at `offset` from the block device.
    ///
    /// Returns how many bytes were read.
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize>;

    //Reads a block from the block device.
    fn read_block(&self, bid: usize, block: &VmFrame) -> Result<()>;

    //Reads a block from the block device.
    fn read_page(&self, bid: usize, block: &VmFrame) -> Result<()>;

    /// Writes a `[u8]` slice at `offset` into the block device.
    ///
    /// Returns how many bytes were written.
    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize>;

    // Writes a block into the block device.
    fn write_block(&self, bid: usize, block: &VmFrame) -> Result<()>;

    //Reads a block from the block device.
    fn write_page(&self, bid: usize, block: &VmFrame) -> Result<()>;
}

pub const BLOCK_SIZE : usize = 512;

impl dyn BlockDevice {
    /// Downcast to the specific type.
    pub fn downcast_ref<T: BlockDevice>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    /// Returns the number of bytes.
    pub fn bytes_count(&self) -> usize {
        self.blocks_count() * self.block_size()
    }

    /// Returns the block_size.
    pub fn block_size(&self) -> usize {
        //TODO: block size should be the same as the sector size.
        BLOCK_SIZE
    }
}

pub fn is_block_aligned(offset: usize) -> bool {
    offset % BLOCK_SIZE == 0
}

impl Debug for dyn BlockDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BlockDevice")
            .field("blocks_count", &self.blocks_count())
            .finish()
    }
}

impl VmIo for dyn BlockDevice {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> jinux_frame::Result<()> {
        if offset >= self.bytes_count() || offset + buf.len() > self.bytes_count() {
            return Err(jinux_frame::Error::InvalidArgs);
        }

        let read_len = self.read_at(offset, buf)?;
        if read_len != buf.len() {
            return Err(jinux_frame::Error::IoError);
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> jinux_frame::Result<()> {
        if offset >= self.bytes_count() || offset + buf.len() > self.bytes_count() {
            return Err(jinux_frame::Error::InvalidArgs);
        }

        let write_len = self.write_at(offset, buf)?;
        if write_len != buf.len() {
            return Err(jinux_frame::Error::IoError);
        }
        Ok(())
    }
}