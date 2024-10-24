// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{FallibleVmRead, FallibleVmWrite, VmIo, VmReader, VmWriter};

use super::{
    bio::{Bio, BioEnqueueError, BioSegment, BioStatus, BioType, BioWaiter, SubmittedBio},
    id::{Bid, Sid},
    BlockDevice, BLOCK_SIZE, SECTOR_SIZE,
};
use crate::{bio::BioDirection, prelude::*};

/// Implements several commonly used APIs for the block device to conveniently
/// read and write block(s).
// TODO: Add API to submit bio with multiple segments in scatter/gather manner.
impl dyn BlockDevice {
    /// Synchronously reads contiguous blocks starting from the `bid`.
    pub fn read_blocks(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
    ) -> Result<BioStatus, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Read,
            Sid::from(bid),
            vec![bio_segment],
            Some(general_complete_fn),
        );
        let status = bio.submit_and_wait(self)?;
        Ok(status)
    }

    /// Asynchronously reads contiguous blocks starting from the `bid`.
    pub fn read_blocks_async(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
    ) -> Result<BioWaiter, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Read,
            Sid::from(bid),
            vec![bio_segment],
            Some(general_complete_fn),
        );
        bio.submit(self)
    }

    /// Synchronously writes contiguous blocks starting from the `bid`.
    pub fn write_blocks(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
    ) -> Result<BioStatus, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Write,
            Sid::from(bid),
            vec![bio_segment],
            Some(general_complete_fn),
        );
        let status = bio.submit_and_wait(self)?;
        Ok(status)
    }

    /// Asynchronously writes contiguous blocks starting from the `bid`.
    pub fn write_blocks_async(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
    ) -> Result<BioWaiter, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Write,
            Sid::from(bid),
            vec![bio_segment],
            Some(general_complete_fn),
        );
        bio.submit(self)
    }
}

impl VmIo for dyn BlockDevice {
    /// Reads consecutive bytes of several sectors in size.
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        let read_len = writer.avail();
        if offset % SECTOR_SIZE != 0 || read_len % SECTOR_SIZE != 0 {
            return Err(ostd::Error::InvalidArgs);
        }
        if read_len == 0 {
            return Ok(());
        }

        let (bio, bio_segment) = {
            let num_blocks = {
                let first = Bid::from_offset(offset).to_raw();
                let last = Bid::from_offset(offset + read_len - 1).to_raw();
                (last - first + 1) as usize
            };
            let bio_segment = BioSegment::from_parts(
                num_blocks,
                offset % BLOCK_SIZE,
                read_len,
                BioDirection::Read,
            );

            (
                Bio::new(
                    BioType::Read,
                    Sid::from_offset(offset),
                    vec![bio_segment.clone()],
                    None,
                ),
                bio_segment,
            )
        };

        let status = bio.submit_and_wait(self)?;
        match status {
            BioStatus::Complete => {
                let _ = bio_segment
                    .reader()
                    .read_fallible(writer)
                    .map_err(|(e, _)| e)?;
                Ok(())
            }
            _ => Err(ostd::Error::IoError),
        }
    }

    /// Writes consecutive bytes of several sectors in size.
    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        let write_len = reader.remain();
        if offset % SECTOR_SIZE != 0 || write_len % SECTOR_SIZE != 0 {
            return Err(ostd::Error::InvalidArgs);
        }
        if write_len == 0 {
            return Ok(());
        }

        let bio = {
            let num_blocks = {
                let first = Bid::from_offset(offset).to_raw();
                let last = Bid::from_offset(offset + write_len - 1).to_raw();
                (last - first + 1) as usize
            };
            let bio_segment = BioSegment::from_parts(
                num_blocks,
                offset % BLOCK_SIZE,
                write_len,
                BioDirection::Write,
            );
            bio_segment
                .writer()
                .to_fallible()
                .write_fallible(reader)
                .map_err(|(e, _)| e)?;
            Bio::new(
                BioType::Write,
                Sid::from_offset(offset),
                vec![bio_segment],
                None,
            )
        };

        let status = bio.submit_and_wait(self)?;
        match status {
            BioStatus::Complete => Ok(()),
            _ => Err(ostd::Error::IoError),
        }
    }
}

impl dyn BlockDevice {
    /// Asynchronously writes consecutive bytes of several sectors in size.
    pub fn write_bytes_async(&self, offset: usize, buf: &[u8]) -> ostd::Result<BioWaiter> {
        if offset % SECTOR_SIZE != 0 || buf.len() % SECTOR_SIZE != 0 {
            return Err(ostd::Error::InvalidArgs);
        }
        if buf.is_empty() {
            return Ok(BioWaiter::new());
        }

        let bio = {
            let num_blocks = {
                let first = Bid::from_offset(offset).to_raw();
                let last = Bid::from_offset(offset + buf.len() - 1).to_raw();
                (last - first + 1) as usize
            };
            let bio_segment = BioSegment::from_parts(
                num_blocks,
                offset % BLOCK_SIZE,
                buf.len(),
                BioDirection::Write,
            );
            bio_segment
                .writer()
                .to_fallible()
                .write_fallible(&mut VmReader::from(buf))
                .map_err(|(e, _)| e)?;
            Bio::new(
                BioType::Write,
                Sid::from_offset(offset),
                vec![bio_segment],
                Some(general_complete_fn),
            )
        };

        let complete = bio.submit(self)?;
        Ok(complete)
    }
}

fn general_complete_fn(bio: &SubmittedBio) {
    match bio.status() {
        BioStatus::Complete => (),
        err_status => log::error!(
            "failed to do {:?} on the device with error status: {:?}",
            bio.type_(),
            err_status
        ),
    }
}
