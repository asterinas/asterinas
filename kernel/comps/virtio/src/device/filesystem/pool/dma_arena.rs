// SPDX-License-Identifier: MPL-2.0

//! DMA arena allocation for large virtio-fs buffers.
//!
//! This module provides [`DmaArenaAllocator`], which serves page-granular
//! allocations from a direction-specific DMA region. Each [`DmaArena`] returns
//! its range automatically when dropped. The parent buffer pool falls back to
//! an independent DMA stream when the allocator cannot satisfy a request.

use alloc::sync::Arc;
use core::ops::Range;

use bitvec::array::BitArray;
use ostd::{
    Result,
    mm::{
        HasDaddr, HasSize, Infallible, PAGE_SIZE, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
    sync::{LocalIrqDisabled, SpinLock},
};

/// Preserves the previous worst-case budget of eight cached 1-MiB streams.
const POOL_SIZE_BYTES: usize = 8 * 1024 * 1024;

/// Allows one allocation to consume the entire arena; larger requests fall back
/// to independently allocated DMA streams.
const MAX_ALLOCATION_SIZE_BYTES: usize = POOL_SIZE_BYTES;

/// The number of page-sized allocation units tracked by the bitmap.
const NUM_PAGES: usize = POOL_SIZE_BYTES / PAGE_SIZE;

/// A page-granular allocator backed by one preallocated DMA stream.
#[derive(Debug)]
pub(super) struct DmaArenaAllocator<D: DmaDirection> {
    storage: DmaStream<D>,
    manager: SpinLock<Manager, LocalIrqDisabled>,
}

#[derive(Debug)]
struct Manager {
    /// A set bit denotes a page owned by a live [`DmaArena`].
    occupied: BitArray<[u8; NUM_PAGES.div_ceil(8)]>,
    /// The lowest free page, or [`NUM_PAGES`] if the arena is full.
    min_free: usize,
}

impl<D: DmaDirection> DmaArenaAllocator<D> {
    pub(super) fn new() -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            storage: DmaStream::alloc_uninit(NUM_PAGES, false)?,
            manager: SpinLock::new(Manager {
                occupied: BitArray::ZERO,
                min_free: 0,
            }),
        }))
    }

    pub(super) fn alloc(self: &Arc<Self>, pages: usize) -> Option<DmaArena<D>> {
        if pages == 0 || pages > MAX_ALLOCATION_SIZE_BYTES / PAGE_SIZE {
            return None;
        }

        let mut manager = self.manager.lock();
        if pages > NUM_PAGES - manager.min_free {
            return None;
        }

        let previous_min_free = manager.min_free;
        let (start, end) = {
            let mut start = previous_min_free;
            let mut end = start;
            while end < NUM_PAGES && end - start < pages {
                if manager.occupied[end] {
                    start = end + 1;
                    end = start;
                } else {
                    end += 1;
                }
            }
            if end - start < pages {
                return None;
            }
            (start, end)
        };

        manager.occupied[start..end].fill(true);
        manager.min_free = manager.occupied[previous_min_free..]
            .iter()
            .position(|occupied| !occupied)
            .map(|position| previous_min_free + position)
            .unwrap_or(NUM_PAGES);

        Some(DmaArena {
            allocator: self.clone(),
            page_range: start..end,
        })
    }

    fn free(&self, page_range: Range<usize>) {
        let mut manager = self.manager.lock();
        debug_assert!(manager.occupied[page_range.clone()].iter().all(|bit| *bit));
        manager.occupied[page_range.clone()].fill(false);
        manager.min_free = manager.min_free.min(page_range.start);
    }
}

/// A variable-length allocation from a [`DmaArenaAllocator`].
///
/// `page_range` remains marked as occupied until this object is dropped.
#[derive(Debug)]
pub(in crate::device::filesystem) struct DmaArena<D: DmaDirection> {
    allocator: Arc<DmaArenaAllocator<D>>,
    page_range: Range<usize>,
}

impl<D: DmaDirection> DmaArena<D> {
    fn byte_range(&self) -> Range<usize> {
        self.page_range.start * PAGE_SIZE..self.page_range.end * PAGE_SIZE
    }

    pub(super) fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        let offset = self.byte_range().start;
        self.allocator
            .storage
            .sync_from_device(byte_range.start + offset..byte_range.end + offset)
    }

    pub(super) fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        let offset = self.byte_range().start;
        self.allocator
            .storage
            .sync_to_device(byte_range.start + offset..byte_range.end + offset)
    }
}

impl<D: DmaDirection> Drop for DmaArena<D> {
    fn drop(&mut self) {
        self.allocator.free(self.page_range.clone());
    }
}

impl<D: DmaDirection> HasSize for DmaArena<D> {
    fn size(&self) -> usize {
        self.page_range.len() * PAGE_SIZE
    }
}

impl<D: DmaDirection> HasDaddr for DmaArena<D> {
    fn daddr(&self) -> ostd::mm::Daddr {
        self.allocator.storage.daddr() + self.byte_range().start
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaArena<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        let byte_range = self.byte_range();
        let mut reader = self.allocator.storage.reader()?;
        reader.skip(byte_range.start).limit(byte_range.len());
        Ok(reader)
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        let byte_range = self.byte_range();
        let mut writer = self.allocator.storage.writer()?;
        writer.skip(byte_range.start).limit(byte_range.len());
        Ok(writer)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::dma::FromDevice, prelude::*};

    use super::*;

    #[ktest]
    fn dropped_pages_are_reused() {
        let allocator = DmaArenaAllocator::<FromDevice>::new().unwrap();
        let segment = allocator.alloc(3).unwrap();
        let daddr = segment.daddr();
        assert_eq!(segment.size(), 3 * PAGE_SIZE);
        drop(segment);

        let reused_segment = allocator.alloc(3).unwrap();
        assert_eq!(reused_segment.daddr(), daddr);
    }

    #[ktest]
    fn entire_arena_can_be_allocated() {
        let allocator = DmaArenaAllocator::<FromDevice>::new().unwrap();
        let arena = allocator.alloc(NUM_PAGES).unwrap();
        assert_eq!(arena.size(), POOL_SIZE_BYTES);
        assert!(allocator.alloc(1).is_none());
        drop(arena);

        assert!(allocator.alloc(NUM_PAGES).is_some());
    }

    #[ktest]
    fn skipped_free_range_remains_allocatable() {
        let allocator = DmaArenaAllocator::<FromDevice>::new().unwrap();
        let first = allocator.alloc(2).unwrap();
        let first_daddr = first.daddr();
        let _barrier = allocator.alloc(1).unwrap();
        let _tail = allocator.alloc(3).unwrap();
        drop(first);

        let _larger_than_gap = allocator.alloc(3).unwrap();
        let reused_gap = allocator.alloc(2).unwrap();
        assert_eq!(reused_gap.daddr(), first_daddr);
    }
}
