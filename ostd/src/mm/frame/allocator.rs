// SPDX-License-Identifier: MPL-2.0

//! The physical memory allocator.

use align_ext::AlignExt;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use super::{meta::AnyFrameMeta, segment::Segment, Frame};
use crate::{
    boot::memory_region::MemoryRegionType,
    error::Error,
    mm::{paddr_to_vaddr, Paddr, PAGE_SIZE},
    prelude::*,
    sync::SpinLock,
};

/// Options for allocating physical memory frames.
pub struct FrameAllocOptions {
    zeroed: bool,
}

impl Default for FrameAllocOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAllocOptions {
    /// Creates new options for allocating the specified number of frames.
    pub fn new() -> Self {
        Self { zeroed: true }
    }

    /// Sets whether the allocated frames should be initialized with zeros.
    ///
    /// If `zeroed` is `true`, the allocated frames are filled with zeros.
    /// If not, the allocated frames will contain sensitive data and the caller
    /// should clear them before sharing them with other components.
    ///
    /// By default, the frames are zero-initialized.
    pub fn zeroed(&mut self, zeroed: bool) -> &mut Self {
        self.zeroed = zeroed;
        self
    }

    /// Allocates a single untyped frame without metadata.
    pub fn alloc_frame(&self) -> Result<Frame<()>> {
        self.alloc_frame_with(())
    }

    /// Allocates a single frame with additional metadata.
    pub fn alloc_frame_with<M: AnyFrameMeta>(&self, metadata: M) -> Result<Frame<M>> {
        let frame = FRAME_ALLOCATOR
            .get()
            .unwrap()
            .disable_irq()
            .lock()
            .alloc(1)
            .map(|idx| {
                let paddr = idx * PAGE_SIZE;
                Frame::from_unused(paddr, metadata).unwrap()
            })
            .ok_or(Error::NoMemory)?;

        if self.zeroed {
            let addr = paddr_to_vaddr(frame.start_paddr()) as *mut u8;
            // SAFETY: The newly allocated frame is guaranteed to be valid.
            unsafe { core::ptr::write_bytes(addr, 0, PAGE_SIZE) }
        }

        Ok(frame)
    }

    /// Allocates a contiguous range of untyped frames without metadata.
    pub fn alloc_segment(&self, nframes: usize) -> Result<Segment<()>> {
        self.alloc_segment_with(nframes, |_| ())
    }

    /// Allocates a contiguous range of frames with additional metadata.
    ///
    /// The returned [`Segment`] contains at least one frame. The method returns
    /// an error if the number of frames is zero.
    pub fn alloc_segment_with<M: AnyFrameMeta, F>(
        &self,
        nframes: usize,
        metadata_fn: F,
    ) -> Result<Segment<M>>
    where
        F: FnMut(Paddr) -> M,
    {
        if nframes == 0 {
            return Err(Error::InvalidArgs);
        }
        let segment = FRAME_ALLOCATOR
            .get()
            .unwrap()
            .disable_irq()
            .lock()
            .alloc(nframes)
            .map(|start| {
                Segment::from_unused(
                    start * PAGE_SIZE..start * PAGE_SIZE + nframes * PAGE_SIZE,
                    metadata_fn,
                )
            })
            .ok_or(Error::NoMemory)?;

        if self.zeroed {
            let addr = paddr_to_vaddr(segment.start_paddr()) as *mut u8;
            // SAFETY: The newly allocated segment is guaranteed to be valid.
            unsafe { core::ptr::write_bytes(addr, 0, nframes * PAGE_SIZE) }
        }

        Ok(segment)
    }
}

#[cfg(ktest)]
#[ktest]
fn test_alloc_dealloc() {
    // Here we allocate and deallocate frames in random orders to test the allocator.
    // We expect the test to fail if the underlying implementation panics.
    let single_options = FrameAllocOptions::new();
    let mut contiguous_options = FrameAllocOptions::new();
    contiguous_options.zeroed(false);
    let mut remember_vec = Vec::new();
    for _ in 0..10 {
        for i in 0..10 {
            let single_frame = single_options.alloc_frame().unwrap();
            if i % 3 == 0 {
                remember_vec.push(single_frame);
            }
        }
        let contiguous_segment = contiguous_options.alloc_segment(10).unwrap();
        drop(contiguous_segment);
        remember_vec.pop();
    }
}

/// FrameAllocator with a counter for allocated memory
pub(in crate::mm) struct CountingFrameAllocator {
    allocator: FrameAllocator,
    total: usize,
    allocated: usize,
}

impl CountingFrameAllocator {
    pub fn new(allocator: FrameAllocator, total: usize) -> Self {
        CountingFrameAllocator {
            allocator,
            total,
            allocated: 0,
        }
    }

    pub fn alloc(&mut self, count: usize) -> Option<usize> {
        match self.allocator.alloc(count) {
            Some(value) => {
                self.allocated += count * PAGE_SIZE;
                Some(value)
            }
            None => None,
        }
    }

    // TODO: this method should be marked unsafe as invalid arguments will mess
    // up the underlying allocator.
    pub fn dealloc(&mut self, start_frame: usize, count: usize) {
        self.allocator.dealloc(start_frame, count);
        self.allocated -= count * PAGE_SIZE;
    }

    pub fn mem_total(&self) -> usize {
        self.total
    }

    pub fn mem_available(&self) -> usize {
        self.total - self.allocated
    }
}

pub(in crate::mm) static FRAME_ALLOCATOR: Once<SpinLock<CountingFrameAllocator>> = Once::new();

pub(crate) fn init() {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let mut total: usize = 0;
    let mut allocator = FrameAllocator::<32>::new();
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned, and skip if it is too small.
            let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
            let region_end = region.base().checked_add(region.len()).unwrap();
            let end = region_end.align_down(PAGE_SIZE) / PAGE_SIZE;
            if end <= start {
                continue;
            }
            // Add global free pages to the frame allocator.
            allocator.add_frame(start, end);
            total += (end - start) * PAGE_SIZE;
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base(),
                region.base() + region.len()
            );
        }
    }
    let counting_allocator = CountingFrameAllocator::new(allocator, total);
    FRAME_ALLOCATOR.call_once(|| SpinLock::new(counting_allocator));
}
