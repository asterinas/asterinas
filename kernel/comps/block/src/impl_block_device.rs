// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{VmIo, VmReader, VmWriter};

use super::{
    BLOCK_SIZE, BlockDevice,
    bio::{
        Bio, BioCompleteFn, BioEnqueueError, BioSegment, BioStatus, BioType, BioWaiter,
        SubmittedBio,
    },
    id::{Bid, Sid},
};
use crate::{
    bio::{BioDirection, is_sector_aligned},
    prelude::*,
};

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
        let bio = Bio::new(BioType::Read, Sid::from(bid), vec![bio_segment], None);
        let status = bio.submit_and_wait(self)?;
        Ok(status)
    }

    /// Asynchronously reads contiguous blocks starting from the `bid`.
    pub fn read_blocks_async(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Read,
            Sid::from(bid),
            vec![bio_segment],
            complete_fn,
        );
        bio.submit(self)
    }

    /// Synchronously writes contiguous blocks starting from the `bid`.
    pub fn write_blocks(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
    ) -> Result<BioStatus, BioEnqueueError> {
        let bio = Bio::new(BioType::Write, Sid::from(bid), vec![bio_segment], None);
        let status = bio.submit_and_wait(self)?;
        Ok(status)
    }

    /// Asynchronously writes contiguous blocks starting from the `bid`.
    pub fn write_blocks_async(
        &self,
        bid: Bid,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter, BioEnqueueError> {
        let bio = Bio::new(
            BioType::Write,
            Sid::from(bid),
            vec![bio_segment],
            complete_fn,
        );
        bio.submit(self)
    }

    /// Issues a sync request
    pub fn sync(&self) -> Result<BioStatus, BioEnqueueError> {
        let bio = Bio::new(BioType::Flush, Sid::from(Bid::from_offset(0)), vec![], None);
        let status = bio.submit_and_wait(self)?;
        Ok(status)
    }
}

impl VmIo for dyn BlockDevice {
    /// Reads consecutive bytes of several sectors in size.
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        let read_len = writer.avail();
        if !is_sector_aligned(offset) || !is_sector_aligned(read_len) {
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
            let bio_segment = BioSegment::alloc_inner(
                num_blocks,
                offset % BLOCK_SIZE,
                read_len,
                BioDirection::FromDevice,
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
            BioStatus::Complete => bio_segment.read(0, writer),
            _ => Err(ostd::Error::IoError),
        }
    }

    /// Writes consecutive bytes of several sectors in size.
    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        let write_len = reader.remain();
        if !is_sector_aligned(offset) || !is_sector_aligned(write_len) {
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
            let bio_segment = BioSegment::alloc_inner(
                num_blocks,
                offset % BLOCK_SIZE,
                write_len,
                BioDirection::ToDevice,
            );
            bio_segment.write(0, reader)?;

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
        let write_len = buf.len();
        if !is_sector_aligned(offset) || !is_sector_aligned(write_len) {
            return Err(ostd::Error::InvalidArgs);
        }
        if write_len == 0 {
            return Ok(BioWaiter::new());
        }

        let bio = {
            let num_blocks = {
                let first = Bid::from_offset(offset).to_raw();
                let last = Bid::from_offset(offset + write_len - 1).to_raw();
                (last - first + 1) as usize
            };
            let bio_segment = BioSegment::alloc_inner(
                num_blocks,
                offset % BLOCK_SIZE,
                write_len,
                BioDirection::ToDevice,
            );
            bio_segment.write(0, &mut VmReader::from(buf).to_fallible())?;
            Bio::new(
                BioType::Write,
                Sid::from_offset(offset),
                vec![bio_segment],
                None,
            )
        };

        let complete = bio.submit(self)?;
        Ok(complete)
    }
}

pub(super) fn general_complete_fn(bio: &SubmittedBio, complete_fn: Option<BioCompleteFn>) {
    let is_success = bio.status() == BioStatus::Complete;
    if !is_success {
        log::error!(
            "failed to do {:?} on the device with error status: {:?}",
            bio.type_(),
            bio.status()
        );
    }
    if let Some(complete_fn) = complete_fn {
        complete_fn(is_success);
    }
}
