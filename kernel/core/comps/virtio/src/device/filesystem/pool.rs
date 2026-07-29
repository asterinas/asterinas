// SPDX-License-Identifier: MPL-2.0

//! Size-classed DMA buffer allocation.
//!
//! This module provides `SizeClassedDmaPool`, a size-class allocator backed by
//! [`DmaPool`] segments for small buffers and [`DmaStream`] for large ones.

use alloc::sync::Arc;
use core::ops::Range;

use aster_network::dma_pool::{DmaPool, DmaSegment};
use aster_util::mem_obj_slice::Slice;
use ostd::{
    Result,
    mm::{
        HasDaddr, HasSize, Infallible, PAGE_SIZE, USegment, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
};

use crate::dma_buf::DmaBuf;

/// Pool-backed buffers start at 64 bytes to avoid wasting a page for small
/// fixed-size buffers.
const MIN_SHIFT: usize = 6;

/// Pool-backed buffers stop at one page. Larger buffers use `DmaStream` so large
/// buffers do not consume all small-buffer pool segments.
const MAX_SHIFT: usize = 12;

/// The number of pool-backed size classes.
const N_CLASSES: usize = MAX_SHIFT - MIN_SHIFT + 1;

/// The largest buffer size served by a pooled DMA segment.
const MAX_CLASS_SIZE: usize = 1 << MAX_SHIFT;

/// Preallocate a few segments per size class to avoid frequent allocation under
/// light concurrency.
const POOL_INIT_SIZE: usize = 8;

/// Retains enough free segments for request bursts.
const POOL_HIGH_WATERMARK: usize = 64;

/// A size-classed DMA buffer allocator.
#[derive(Debug)]
pub(super) struct SizeClassedDmaPool<D: DmaDirection> {
    classes: [Arc<DmaPool<D>>; N_CLASSES],
}

impl<D: DmaDirection> SizeClassedDmaPool<D> {
    /// Creates a DMA buffer pool with predefined size classes.
    pub(super) fn new() -> Arc<Self> {
        let classes = core::array::from_fn(|i| {
            let segment_size = 1 << (MIN_SHIFT + i);
            DmaPool::<D>::new(segment_size, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false)
        });
        Arc::new(Self { classes })
    }

    /// Allocates a DMA buffer whose visible length is `len`.
    fn alloc_buf(&self, len: usize) -> Result<Arc<Slice<FsDmaStorage<D>>>> {
        if len == 0 {
            return Err(ostd::Error::InvalidArgs);
        }

        let storage = if len <= MAX_CLASS_SIZE {
            let shift = MIN_SHIFT.max(len.next_power_of_two().trailing_zeros() as usize);
            let segment = self.classes[shift - MIN_SHIFT].alloc_segment()?;
            FsDmaStorage::Segment(segment)
        } else {
            let stream = DmaStream::alloc_uninit(len.div_ceil(PAGE_SIZE), false)?;
            FsDmaStorage::Stream(stream)
        };

        Ok(Arc::new(Slice::new(storage, 0..len)))
    }
}

impl SizeClassedDmaPool<FromDevice> {
    /// Allocates a DMA buffer for FUSE reply payloads.
    pub(super) fn alloc_reply_buf(&self, len: usize) -> Result<FuseReplyBuf> {
        self.alloc_buf(len).map(FuseReplyBuf)
    }
}

impl SizeClassedDmaPool<ToDevice> {
    /// Allocates a DMA buffer for FUSE requests.
    pub(super) fn alloc_request_buf(&self, len: usize) -> Result<FuseRequestBuf> {
        self.alloc_buf(len).map(FuseRequestBuf)
    }
}

/// A data payload buffer used by FUSE I/O operations.
pub(super) enum FuseDataBuf {
    /// Data filled by the device for read FUSE operations.
    Read(FuseReplyBuf),
    /// Data sent to the device for write FUSE operations.
    Write(FuseRequestBuf),
}

/// A DMA buffer used by FUSE requests.
#[derive(Clone, Debug)]
pub struct FuseRequestBuf(Arc<Slice<FsDmaStorage<ToDevice>>>);

impl FuseRequestBuf {
    /// Returns the length of the DMA buffer.
    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    /// Returns the DMA slice used by virtqueue descriptors.
    pub(crate) fn as_dma_slice(&self) -> &Slice<FsDmaStorage<ToDevice>> {
        self.0.as_ref()
    }

    /// Synchronizes the whole buffer from memory to the device.
    pub(crate) fn sync_to_device(&self) -> Result<()> {
        self.0.mem_obj().sync_to_device(self.0.offset().clone())
    }
}

impl HasVmReaderWriter for FuseRequestBuf {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        self.0.reader()
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        self.0.writer()
    }
}

/// A DMA buffer used by FUSE replies.
#[derive(Clone, Debug)]
pub struct FuseReplyBuf(Arc<Slice<FsDmaStorage<FromDevice>>>);

impl FuseReplyBuf {
    /// Maps `segment` as a DMA buffer for FUSE reply payloads.
    pub fn new_map(segment: USegment) -> Result<Self> {
        let len = segment.size();
        let stream = DmaStream::map(segment, false)?;

        Ok(FuseReplyBuf(Arc::new(Slice::new(
            FsDmaStorage::Stream(stream),
            0..len,
        ))))
    }

    /// Returns the length of the DMA buffer.
    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    /// Returns the DMA slice used by virtqueue descriptors.
    pub(crate) fn as_dma_slice(&self) -> &Slice<FsDmaStorage<FromDevice>> {
        self.0.as_ref()
    }

    /// Synchronizes the whole buffer from the device into memory.
    pub(crate) fn sync_from_device(&self) -> Result<()> {
        self.0.mem_obj().sync_from_device(self.0.offset().clone())
    }
}

impl HasVmReaderWriter for FuseReplyBuf {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        self.0.reader()
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        self.0.writer()
    }
}

/// The backing storage for a virtio-fs DMA buffer.
#[derive(Debug)]
pub(crate) enum FsDmaStorage<D: DmaDirection> {
    /// A contiguous DMA stream for large buffers.
    Stream(DmaStream<D>),
    /// A pooled DMA segment for small buffers.
    Segment(DmaSegment<D>),
}

impl<D: DmaDirection> FsDmaStorage<D> {
    /// Synchronizes `byte_range` from the device into memory.
    pub(super) fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Stream(stream) => stream.sync_from_device(byte_range),
            Self::Segment(segment) => segment.sync_from_device(byte_range),
        }
    }

    /// Synchronizes `byte_range` from memory to the device.
    pub(super) fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Stream(stream) => stream.sync_to_device(byte_range),
            Self::Segment(segment) => segment.sync_to_device(byte_range),
        }
    }
}

impl<D: DmaDirection> HasSize for FsDmaStorage<D> {
    fn size(&self) -> usize {
        match self {
            Self::Stream(stream) => stream.size(),
            Self::Segment(segment) => segment.size(),
        }
    }
}

impl<D: DmaDirection> HasDaddr for FsDmaStorage<D> {
    fn daddr(&self) -> ostd::mm::Daddr {
        match self {
            Self::Stream(stream) => stream.daddr(),
            Self::Segment(segment) => segment.daddr(),
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for FsDmaStorage<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        match self {
            Self::Stream(stream) => stream.reader(),
            Self::Segment(segment) => segment.reader(),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        match self {
            Self::Stream(stream) => stream.writer(),
            Self::Segment(segment) => segment.writer(),
        }
    }
}

impl<D: DmaDirection> DmaBuf for Slice<FsDmaStorage<D>> {
    fn len(&self) -> usize {
        self.size()
    }
}
