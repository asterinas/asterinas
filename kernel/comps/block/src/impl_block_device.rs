// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use io_util::batch::IoBatch;
use ostd::mm::{VmIo, VmReader, VmWriter};

use super::{
    BLOCK_SIZE, BlockDevice, SECTOR_SIZE,
    bio::{Bio, BioCompleteFn, BioEnqueueError, BioSegment, BioStatus, BioType},
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
        io_batch: &mut IoBatch,
    ) -> Result<(), BioEnqueueError> {
        let bio = Bio::new(
            BioType::Read,
            Sid::from(bid),
            vec![bio_segment],
            complete_fn,
        );
        bio.submit(self, io_batch)
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
        io_batch: &mut IoBatch,
    ) -> Result<(), BioEnqueueError> {
        let bio = Bio::new(
            BioType::Write,
            Sid::from(bid),
            vec![bio_segment],
            complete_fn,
        );
        bio.submit(self, io_batch)
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
        if read_len == 0 {
            return Ok(());
        }

        let request_end = offset.checked_add(read_len).ok_or(ostd::Error::Overflow)?;
        let device_size = self.metadata().nr_sectors * SECTOR_SIZE;
        if request_end > device_size {
            return Err(ostd::Error::InvalidArgs);
        }

        let aligned_offset = offset.align_down(SECTOR_SIZE);
        let aligned_end = request_end.align_up(SECTOR_SIZE);
        let aligned_len = aligned_end - aligned_offset;

        let (bio, bio_segment) = {
            let num_blocks = {
                let first = Bid::from_offset(aligned_offset).to_raw();
                let last = Bid::from_offset(aligned_end - 1).to_raw();
                (last - first + 1) as usize
            };
            let bio_segment = BioSegment::alloc_inner(
                num_blocks,
                aligned_offset % BLOCK_SIZE,
                aligned_len,
                BioDirection::FromDevice,
            );

            (
                Bio::new(
                    BioType::Read,
                    Sid::from_offset(aligned_offset),
                    vec![bio_segment.clone()],
                    None,
                ),
                bio_segment,
            )
        };

        let status = bio.submit_and_wait(self)?;
        match status {
            BioStatus::Complete => {
                let segment_offset = offset - aligned_offset;
                bio_segment.read(segment_offset, writer)?;

                Ok(())
            }
            _ => Err(ostd::Error::IoError),
        }
    }

    /// Writes consecutive bytes of several sectors in size.
    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(());
        }

        let request_end = offset.checked_add(write_len).ok_or(ostd::Error::Overflow)?;
        let device_size = self.metadata().nr_sectors * SECTOR_SIZE;
        if request_end > device_size {
            return Err(ostd::Error::InvalidArgs);
        }

        let aligned_offset = offset.align_down(SECTOR_SIZE);
        let aligned_end = request_end.align_up(SECTOR_SIZE);

        // If the write range is not sector-aligned, preserve the bytes in the
        // surrounding sectors that are outside the user-requested range.
        // The request is split into at most three segments: a read-modify-write
        // first sector, sector-aligned middle sectors written directly from the
        // reader, and a read-modify-write last sector. Each segment consumes
        // only the bytes that belong to it so later segments see the remaining
        // input bytes.

        let need_read_first_sector = !is_sector_aligned(offset);
        let mut middle_sector_offset = aligned_offset;
        let last_sector_offset = (request_end - 1).align_down(SECTOR_SIZE);
        let need_read_last_sector = {
            let is_last_sector_aligned = is_sector_aligned(request_end);
            let is_the_same_sector = last_sector_offset == aligned_offset;
            !(is_last_sector_aligned || need_read_first_sector && is_the_same_sector)
        };
        let middle_end = if need_read_last_sector {
            last_sector_offset
        } else {
            aligned_end
        };

        let mut bio_segments = Vec::new();

        if need_read_first_sector {
            let first_segment = self.read_sector_for_write(aligned_offset)?;
            let first_segment_offset = offset - aligned_offset;
            let first_write_len = (SECTOR_SIZE - first_segment_offset).min(write_len);
            let mut first_reader = reader.clone();
            first_reader.limit(first_write_len);
            first_segment.write(first_segment_offset, &mut first_reader)?;
            reader.skip(first_write_len);
            bio_segments.push(first_segment);
            middle_sector_offset += SECTOR_SIZE;
        }
        if middle_sector_offset < middle_end {
            let middle_len = middle_end - middle_sector_offset;
            let middle_segment = alloc_write_segment(middle_sector_offset, middle_len);
            let mut middle_reader = reader.clone();
            middle_reader.limit(middle_len);
            middle_segment.write(0, &mut middle_reader)?;
            reader.skip(middle_len);
            bio_segments.push(middle_segment);
        }
        if need_read_last_sector {
            let last_segment = self.read_sector_for_write(last_sector_offset)?;
            last_segment.write(0, reader)?;
            bio_segments.push(last_segment);
        }
        debug_assert!(!reader.has_remain());

        let bio = Bio::new(
            BioType::Write,
            Sid::from_offset(aligned_offset),
            bio_segments,
            None,
        );
        let status = bio.submit_and_wait(self)?;
        match status {
            BioStatus::Complete => Ok(()),
            _ => Err(ostd::Error::IoError),
        }
    }
}

impl dyn BlockDevice {
    /// Asynchronously writes consecutive bytes of several sectors in size.
    pub fn write_bytes_async(
        &self,
        offset: usize,
        buf: &[u8],
        io_batch: &mut IoBatch,
    ) -> ostd::Result<()> {
        let write_len = buf.len();
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
            bio_segment.write(0, &mut VmReader::from(buf).to_fallible())?;
            Bio::new(
                BioType::Write,
                Sid::from_offset(offset),
                vec![bio_segment],
                None,
            )
        };

        bio.submit(self, io_batch)?;
        Ok(())
    }

    fn read_sector_for_write(&self, sector_offset: usize) -> ostd::Result<BioSegment> {
        // The segment will be submitted by the later write bio, so keep it writable
        // from the CPU side and read the preserved sector directly into it.
        let write_segment = alloc_write_segment(sector_offset, SECTOR_SIZE);
        let read_bio = Bio::new(
            BioType::Read,
            Sid::from_offset(sector_offset),
            vec![write_segment.clone()],
            None,
        );
        if read_bio.submit_and_wait(self)? != BioStatus::Complete {
            return Err(ostd::Error::IoError);
        }

        Ok(write_segment)
    }
}

fn alloc_write_segment(offset: usize, len: usize) -> BioSegment {
    let num_blocks = {
        let first = Bid::from_offset(offset).to_raw();
        let last = Bid::from_offset(offset + len - 1).to_raw();
        (last - first + 1) as usize
    };

    BioSegment::alloc_inner(num_blocks, offset % BLOCK_SIZE, len, BioDirection::ToDevice)
}

pub(super) fn general_complete_fn(
    bio_type: BioType,
    bio_status: BioStatus,
    complete_fn: Option<BioCompleteFn>,
) {
    if bio_status != BioStatus::Complete {
        ostd::error!(
            "failed to do {:?} on the device with error status: {:?}",
            bio_type,
            bio_status
        );
    }
    if let Some(complete_fn) = complete_fn {
        complete_fn(bio_status);
    }
}
