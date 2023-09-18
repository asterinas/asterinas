use crate::bid::{BlockId, BLOCK_SIZE};
use crate::bio::{Bio, BioBuf};
use crate::prelude::*;

/// A block device.
pub trait BlockDevice: Send + Sync + Any + Debug {
    fn submit_bio(&self, bio: &mut Bio) -> Result<usize>;

    fn total_blocks(&self) -> BlockId;
}

impl GenericIo for dyn BlockDevice {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut bio = Bio::from_bytes_mut_at(buf, offset);
        let num_processed = self.submit_bio(&mut bio)?;
        if num_processed != bio.bio_bufs().len() {
            return Err(Error::IoError);
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let mut bio = Bio::from_bytes_at(buf, offset);
        let num_processed = self.submit_bio(&mut bio)?;
        if num_processed != bio.bio_bufs().len() {
            return Err(Error::IoError);
        }
        Ok(())
    }
}

pub trait BlockDeviceExt: Send + Sync {
    fn read_block(&self, bid: BlockId, block: &mut BioBuf) -> Result<()>;

    fn write_block(&self, bid: BlockId, block: &BioBuf) -> Result<()>;
}

impl BlockDeviceExt for dyn BlockDevice {
    fn read_block(&self, bid: BlockId, block: &mut BioBuf) -> Result<()> {
        if block.len() != BLOCK_SIZE {
            return Err(Error::InvalidArgs);
        }

        self.read_bytes(bid.to_offset(), block.as_mut_slice())
    }

    fn write_block(&self, bid: BlockId, block: &BioBuf) -> Result<()> {
        if block.len() != BLOCK_SIZE {
            return Err(Error::InvalidArgs);
        }

        self.write_bytes(bid.to_offset(), block.as_slice())
    }
}
