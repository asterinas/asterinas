
use jinux_frame::boot::page_array::PageArray;
use jinux_frame::vm::VmFrame;
use jinux_frame::vm::VmIo;

/// A simple block device for Exfat.
pub trait BlockDevice: Send + Sync + Any {
    /// Returns the number of blocks.
    fn blocks_count(&self) -> Bid;

    /// Reads a `[u8]` slice at `offset` from the block device.
    ///
    /// Returns how many bytes were read.
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize>;

    /// Reads a block from the block device.
    fn read_block(&self, bid: Bid, block: &VmFrame) -> Result<()>;

    /// Writes a `[u8]` slice at `offset` into the block device.
    ///
    /// Returns how many bytes were written.
    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize>;

    /// Writes a block into the block device.
    fn write_block(&self, bid: Bid, block: &VmFrame) -> Result<()>;
}

impl dyn BlockDevice {
    /// Downcast to the specific type.
    pub fn downcast_ref<T: BlockDevice>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    /// Returns the number of bytes.
    pub fn bytes_count(&self) -> usize {
        self.blocks_count().to_raw() as usize * self.block_size()
    }

    /// Returns the block_size.
    pub fn block_size(&self) -> usize {
        512
    }
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