// SPDX-License-Identifier: MPL-2.0

//! Page-granular allocations from a pre-mapped DMA region.

use alloc::sync::Arc;
use core::ops::Range;

use bitvec::vec::BitVec;
use ostd::{
    Result,
    mm::{
        HasDaddr, HasSize, Infallible, PAGE_SIZE, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
    sync::{LocalIrqDisabled, SpinLock},
};

use crate::mem_obj_slice::Slice;

/// A page-granular allocator backed by one pre-mapped DMA stream.
#[derive(Debug)]
pub struct DmaArenaAllocator<D: DmaDirection> {
    storage: Arc<DmaStream<D>>,
    manager: SpinLock<Manager, LocalIrqDisabled>,
}

#[derive(Debug)]
struct Manager {
    /// A set bit denotes a page owned by a live [`DmaArena`].
    occupied: BitVec,
    /// The lowest free page, or the arena capacity if the arena is full.
    min_free: usize,
}

impl<D: DmaDirection> DmaArenaAllocator<D> {
    /// Creates an arena containing `capacity_pages` non-coherent DMA pages.
    pub fn new(capacity_pages: usize) -> Result<Arc<Self>> {
        if capacity_pages == 0 {
            return Err(ostd::Error::InvalidArgs);
        }

        Ok(Arc::new(Self {
            storage: Arc::new(DmaStream::alloc_uninit(capacity_pages, false)?),
            manager: SpinLock::new(Manager {
                occupied: BitVec::repeat(false, capacity_pages),
                min_free: 0,
            }),
        }))
    }

    /// Allocates `size_pages` contiguous pages from the arena.
    pub fn alloc(self: &Arc<Self>, size_pages: usize) -> Option<DmaArena<D>> {
        let mut manager = self.manager.lock();
        let capacity_pages = manager.occupied.len();
        if size_pages == 0 || size_pages > capacity_pages - manager.min_free {
            return None;
        }

        let previous_min_free = manager.min_free;
        let (start, end) = {
            let mut start = previous_min_free;
            let mut end = start;
            while end < capacity_pages && end - start < size_pages {
                if manager.occupied[end] {
                    start = end + 1;
                    end = start;
                } else {
                    end += 1;
                }
            }
            if end - start < size_pages {
                return None;
            }
            (start, end)
        };

        manager.occupied[start..end].fill(true);
        manager.min_free = manager.occupied[previous_min_free..]
            .iter()
            .position(|occupied| !*occupied)
            .map(|position| previous_min_free + position)
            .unwrap_or(capacity_pages);

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
#[derive(Debug)]
pub struct DmaArena<D: DmaDirection> {
    allocator: Arc<DmaArenaAllocator<D>>,
    page_range: Range<usize>,
}

impl<D: DmaDirection> DmaArena<D> {
    fn byte_range(&self) -> Range<usize> {
        self.page_range.start * PAGE_SIZE..self.page_range.end * PAGE_SIZE
    }

    fn absolute_byte_range(&self, byte_range: Range<usize>) -> Result<Range<usize>> {
        if byte_range.start > byte_range.end || byte_range.end > self.size() {
            return Err(ostd::Error::InvalidArgs);
        }

        let arena_start = self.byte_range().start;
        Ok(arena_start + byte_range.start..arena_start + byte_range.end)
    }

    /// Creates a byte slice that retains this allocation until it is dropped.
    pub fn into_slice(self, byte_range: Range<usize>) -> DmaArenaSlice<D> {
        assert!(
            !byte_range.is_empty() && byte_range.end <= self.size(),
            "the byte range must be non-empty and within the DMA arena"
        );

        let arena_start = self.byte_range().start;
        let absolute_range = arena_start + byte_range.start..arena_start + byte_range.end;
        let dma_slice = Slice::new(self.allocator.storage.clone(), absolute_range);
        DmaArenaSlice {
            dma_slice,
            _arena: self,
        }
    }

    /// Synchronizes `byte_range` from the device into memory.
    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        self.allocator
            .storage
            .sync_from_device(self.absolute_byte_range(byte_range)?)
    }

    /// Synchronizes `byte_range` from memory to the device.
    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        self.allocator
            .storage
            .sync_to_device(self.absolute_byte_range(byte_range)?)
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

/// A DMA stream slice that owns its arena allocation.
#[derive(Debug)]
pub struct DmaArenaSlice<D: DmaDirection> {
    dma_slice: Slice<Arc<DmaStream<D>>>,
    _arena: DmaArena<D>,
}

impl<D: DmaDirection> DmaArenaSlice<D> {
    /// Returns the DMA stream slice.
    pub fn dma_slice(&self) -> &Slice<Arc<DmaStream<D>>> {
        &self.dma_slice
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::dma::FromDevice, prelude::*};

    use super::*;

    const CAPACITY_PAGES: usize = 12;

    #[ktest]
    fn dropped_pages_are_reused() {
        let allocator = DmaArenaAllocator::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let segment = allocator.alloc(3).unwrap();
        let daddr = segment.daddr();
        assert_eq!(segment.size(), 3 * PAGE_SIZE);
        drop(segment);

        let reused_segment = allocator.alloc(3).unwrap();
        assert_eq!(reused_segment.daddr(), daddr);
    }

    #[ktest]
    fn entire_arena_can_be_allocated() {
        let allocator = DmaArenaAllocator::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let arena = allocator.alloc(CAPACITY_PAGES).unwrap();
        assert_eq!(arena.size(), CAPACITY_PAGES * PAGE_SIZE);
        assert!(allocator.alloc(1).is_none());
        drop(arena);

        assert!(allocator.alloc(CAPACITY_PAGES).is_some());
    }

    #[ktest]
    fn skipped_free_range_remains_allocatable() {
        let allocator = DmaArenaAllocator::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let first = allocator.alloc(2).unwrap();
        let first_daddr = first.daddr();
        let _barrier = allocator.alloc(1).unwrap();
        let _tail = allocator.alloc(3).unwrap();
        drop(first);

        let _larger_than_gap = allocator.alloc(3).unwrap();
        let reused_gap = allocator.alloc(2).unwrap();
        assert_eq!(reused_gap.daddr(), first_daddr);
    }

    #[ktest]
    fn slice_keeps_pages_allocated() {
        let allocator = DmaArenaAllocator::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let arena_slice = allocator
            .alloc(CAPACITY_PAGES)
            .unwrap()
            .into_slice(0..PAGE_SIZE);
        assert!(allocator.alloc(1).is_none());
        drop(arena_slice);

        assert!(allocator.alloc(CAPACITY_PAGES).is_some());
    }
}
