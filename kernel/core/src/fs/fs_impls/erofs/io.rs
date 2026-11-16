// SPDX-License-Identifier: MPL-2.0

//! Checked device ranges and BIO completion ownership for EROFS reads.
//!
//! Successful reads provide raw bytes, without field or checksum validation.
//! Device capacity must remain stable while this code and `VmIo` query metadata.
//! The underlying BIO allocator's out-of-memory behavior is unchanged.

use aster_block::{
    BlockDevice,
    bio::{Bio, BioCompleteFn, BioSegment, BioStatus, BioType},
    id::Sid,
};
use io_util::batch::IoBatch;
use ostd::mm::{self, VmIo, io::util::HasVmReaderWriter};

use super::ondisk::super_block;
use crate::{
    prelude::{Arc, Box, Errno, Error, Result, SpinLock, vec},
    vm::page_cache::{BlockAsPageCacheBackend, PageCache},
};

/// A device owner independent of filesystem construction.
#[derive(Debug)]
pub(super) struct RawPageBackend {
    device: Arc<dyn BlockDevice>,
    capacity_bytes: usize,
}

impl BlockAsPageCacheBackend for RawPageBackend {
    fn submit_read_bio(
        &self,
        index: usize,
        segment: BioSegment,
        complete_fn: BioCompleteFn,
        batch: &mut IoBatch,
    ) -> Result<()> {
        let (start_bytes, valid_bytes) = match self.page_geometry(index) {
            Ok(geometry) => geometry,
            Err(error) => {
                // Retire DMA ownership before releasing the callback's page lock.
                drop(segment);
                drop(complete_fn);
                return Err(error);
            }
        };
        if valid_bytes == mm::PAGE_SIZE {
            return submit_read_bio(
                self.device.as_ref(),
                start_bytes,
                segment,
                complete_fn,
                batch,
            );
        }

        // The target segment never reaches the device. Exact sector I/O uses
        // an internal buffer, then the CPU initializes this DMA buffer directly.
        let result: Result<()> = (|| {
            let mut writer = segment.dma_slice().writer()?;
            writer.limit(valid_bytes);
            self.device.read(start_bytes, &mut writer.to_fallible())?;
            segment
                .dma_slice()
                .writer()?
                .skip(valid_bytes)
                .fill_zeros(mm::PAGE_SIZE - valid_bytes);
            Ok(())
        })();
        drop(segment);
        result?;
        complete_fn(BioStatus::Complete);
        Ok(())
    }

    fn submit_write_bio(
        &self,
        _index: usize,
        _segment: BioSegment,
        _complete_fn: BioCompleteFn,
        _batch: &mut IoBatch,
    ) -> Result<()> {
        // The generic adapter prepares writeback and rolls it back on refusal.
        Err(Error::new(Errno::EROFS))
    }
}

impl RawPageBackend {
    /// Retains a device with representable capacity for raw page reads.
    pub(super) fn new(device: Arc<dyn BlockDevice>) -> ostd::Result<Self> {
        let capacity_bytes = device
            .metadata()
            .nr_sectors
            .checked_mul(aster_block::SECTOR_SIZE)
            .ok_or(ostd::Error::Overflow)?;
        Ok(Self {
            device,
            capacity_bytes,
        })
    }

    /// Creates a distinct, demand-paged VMO backed by this device owner.
    pub(super) fn create_cache(self: &Arc<Self>) -> ostd::Result<PageCache> {
        // PageCache rounds unchecked; validate its input before construction.
        let aligned_capacity_bytes = self
            .capacity_bytes
            .checked_next_multiple_of(mm::PAGE_SIZE)
            .ok_or(ostd::Error::Overflow)?;
        PageCache::new_with_backend(aligned_capacity_bytes, Arc::downgrade(self) as _)
            .map_err(ostd::Error::from)
    }

    pub(super) fn nr_sectors(&self) -> usize {
        self.capacity_bytes / aster_block::SECTOR_SIZE
    }

    fn page_geometry(&self, index: usize) -> Result<(usize, usize)> {
        let start_bytes = index
            .checked_mul(mm::PAGE_SIZE)
            .ok_or(ostd::Error::Overflow)?;
        if start_bytes >= self.capacity_bytes {
            return Err(Error::new(Errno::EINVAL));
        }
        Ok((
            start_bytes,
            mm::PAGE_SIZE.min(self.capacity_bytes - start_bytes),
        ))
    }
}

/// Submits one checked device range, retaining its callback through refusal cleanup.
///
/// The caller validates the range and supplies any EOF initialization through
/// `complete_fn`. Accepted completion runs after DMA references
/// retire; refused submission releases the callback only after submission returns.
pub(super) fn submit_read_bio(
    device: &dyn BlockDevice,
    start_bytes: usize,
    segment: BioSegment,
    complete_fn: BioCompleteFn,
    batch: &mut IoBatch,
) -> Result<()> {
    // A refused BIO can destroy its callback before its DMA segments. Keep the
    // unconsumed completion alive until submit has cleaned those segments up.
    let completion_slot = Arc::new(SpinLock::new(Some(complete_fn)));
    let callback_slot = completion_slot.clone();
    let callback_fn = Box::new(move |status| {
        let complete_fn = callback_slot.disable_irq().lock().take();
        if let Some(complete_fn) = complete_fn {
            complete_fn(status);
        }
    });
    let result = Bio::new(
        BioType::Read,
        Sid::from_offset(start_bytes),
        vec![segment],
        Some(callback_fn),
    )
    .submit(device, batch)
    .map_err(Error::from);
    if result.is_err() {
        let complete_fn = completion_slot.disable_irq().lock().take();
        drop(complete_fn);
    }
    // Inline completion already consumed the slot. An accepted asynchronous
    // request retains the callback's reference until its completion runs.
    drop(completion_slot);
    result
}

// For the supported 4 KiB format, retain bytes through the first block's end.
// The frozen Linux v6.6 implementation subtracts the superblock offset:
// https://github.com/torvalds/linux/blob/ffc253263a1375a65fa6c9f62a893e9767fbebfa/fs/erofs/super.c#L51
pub(super) const SUPER_BLOCK_COVERAGE_SIZE_BYTES: usize =
    super_block::SUPPORTED_BLOCK_SIZE_BYTES - super_block::SUPER_BLOCK_OFFSET_BYTES;

/// Reads the complete fixed superblock coverage without interpreting it.
pub(super) fn read_superblock_coverage(
    device: &dyn BlockDevice,
) -> ostd::Result<[u8; SUPER_BLOCK_COVERAGE_SIZE_BYTES]> {
    check_read_range(
        device.metadata().nr_sectors,
        super_block::SUPER_BLOCK_OFFSET_BYTES,
        SUPER_BLOCK_COVERAGE_SIZE_BYTES,
    )?;

    let mut bytes = [0; SUPER_BLOCK_COVERAGE_SIZE_BYTES];
    device.read_bytes(super_block::SUPER_BLOCK_OFFSET_BYTES, &mut bytes)?;
    Ok(bytes)
}

/// Checks capacity arithmetic and the request's right-open byte boundary.
pub(super) fn check_read_range(
    nr_sectors: usize,
    offset_bytes: usize,
    length_bytes: usize,
) -> ostd::Result<()> {
    // VmIo's capacity multiplication is unchecked, so reject it before entry.
    let capacity_bytes = nr_sectors
        .checked_mul(aster_block::SECTOR_SIZE)
        .ok_or(ostd::Error::Overflow)?;
    let end_bytes = offset_bytes
        .checked_add(length_bytes)
        .ok_or(ostd::Error::Overflow)?;
    if end_bytes > capacity_bytes {
        return Err(ostd::Error::InvalidArgs);
    }
    Ok(())
}
