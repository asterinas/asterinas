// SPDX-License-Identifier: MPL-2.0

//! Size-classed DMA buffer allocation.
//!
//! This module provides `VirtiofsDmaPool`, a size-class allocator backed by
//! [`DmaPool`] segments for small buffers and a shared DMA arena for large ones.

use alloc::sync::Arc;

use aster_util::mem_obj_slice::Slice;
use dma_pool::{DmaArenaPool, DmaBuffer, DmaPool};
use ostd::{
    Result,
    mm::{
        HasSize, Infallible, PAGE_SIZE, USegment, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
};
use spin::Once;

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

/// Preserves the previous worst-case budget of eight cached 1-MiB streams.
const DMA_ARENA_SIZE_PAGES: usize = 8 * 1024 * 1024 / PAGE_SIZE;

/// A pool of DMA arenas for buffers received from the device.
static VIRTIOFS_DMA_ARENA_RPOOL: Once<Arc<DmaArenaPool<FromDevice>>> = Once::new();
/// A pool of DMA arenas for buffers sent to the device.
static VIRTIOFS_DMA_ARENA_WPOOL: Once<Arc<DmaArenaPool<ToDevice>>> = Once::new();

/// Returns the shared virtio-fs DMA arena pools, initializing them on first use.
pub(super) fn dma_arena_pools_singleton() -> (
    &'static Arc<DmaArenaPool<ToDevice>>,
    &'static Arc<DmaArenaPool<FromDevice>>,
) {
    VIRTIOFS_DMA_ARENA_RPOOL.call_once(|| DmaArenaPool::new(DMA_ARENA_SIZE_PAGES).unwrap());
    VIRTIOFS_DMA_ARENA_WPOOL.call_once(|| DmaArenaPool::new(DMA_ARENA_SIZE_PAGES).unwrap());

    (
        VIRTIOFS_DMA_ARENA_WPOOL.get().unwrap(),
        VIRTIOFS_DMA_ARENA_RPOOL.get().unwrap(),
    )
}

/// A virtio-fs DMA buffer allocator for pooled segments and large arenas.
#[derive(Debug)]
pub(super) struct VirtiofsDmaPool<D: DmaDirection> {
    classes: [Arc<DmaPool<D>>; N_CLASSES],
    arena_pool: &'static Arc<DmaArenaPool<D>>,
}

impl<D: DmaDirection> VirtiofsDmaPool<D> {
    /// Creates a DMA buffer pool with predefined size classes.
    pub(super) fn new(arena_pool: &'static Arc<DmaArenaPool<D>>) -> Self {
        let classes = core::array::from_fn(|i| {
            let segment_size = 1 << (MIN_SHIFT + i);
            DmaPool::<D>::new(segment_size, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false)
        });

        Self {
            classes,
            arena_pool,
        }
    }

    /// Allocates a DMA buffer whose visible length is `len`.
    fn alloc_buf(&self, len: usize) -> Result<Arc<Slice<DmaBuffer<D>>>> {
        if len == 0 {
            return Err(ostd::Error::InvalidArgs);
        }

        let storage = if len <= MAX_CLASS_SIZE {
            let shift = MIN_SHIFT.max(len.next_power_of_two().trailing_zeros() as usize);
            DmaBuffer::Pooled(self.classes[shift - MIN_SHIFT].alloc_segment()?)
        } else {
            let pages = len.div_ceil(PAGE_SIZE);
            match self.arena_pool.alloc(pages) {
                Some(arena) => DmaBuffer::Arena(arena),
                None => DmaBuffer::Direct(DmaStream::alloc_uninit(pages, false)?),
            }
        };

        Ok(Arc::new(Slice::new(storage, 0..len)))
    }
}

impl VirtiofsDmaPool<FromDevice> {
    /// Allocates a DMA buffer for FUSE reply payloads.
    pub(super) fn alloc_reply_buf(&self, len: usize) -> Result<FuseReplyBuf> {
        self.alloc_buf(len).map(FuseReplyBuf)
    }
}

impl VirtiofsDmaPool<ToDevice> {
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
pub struct FuseRequestBuf(Arc<Slice<DmaBuffer<ToDevice>>>);

impl FuseRequestBuf {
    /// Returns the length of the DMA buffer.
    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    /// Returns the DMA slice used by virtqueue descriptors.
    pub(crate) fn as_dma_slice(&self) -> &Slice<DmaBuffer<ToDevice>> {
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
pub struct FuseReplyBuf(Arc<Slice<DmaBuffer<FromDevice>>>);

impl FuseReplyBuf {
    /// Maps `segment` as a DMA buffer for FUSE reply payloads.
    pub fn new_map(segment: USegment) -> Result<Self> {
        let len = segment.size();
        let stream = DmaStream::map(segment, false)?;

        Ok(FuseReplyBuf(Arc::new(Slice::new(
            DmaBuffer::Direct(stream),
            0..len,
        ))))
    }

    /// Returns the length of the DMA buffer.
    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    /// Returns the DMA slice used by virtqueue descriptors.
    pub(crate) fn as_dma_slice(&self) -> &Slice<DmaBuffer<FromDevice>> {
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
