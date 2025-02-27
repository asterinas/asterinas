// SPDX-License-Identifier: MPL-2.0

//! The physical memory allocator.

use core::{
    alloc::Layout,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

use align_ext::AlignExt;

use super::{meta::AnyFrameMeta, segment::Segment, Frame};
use crate::{
    boot::memory_region::{MemoryRegion, MemoryRegionType},
    error::Error,
    mm::{paddr_to_vaddr, Paddr, PAGE_SIZE},
    prelude::*,
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
        let single_layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let frame = alloc_upcall(single_layout)
            .map(|paddr| Frame::from_unused(paddr, metadata).unwrap())
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
        let layout = Layout::from_size_align(nframes * PAGE_SIZE, PAGE_SIZE).unwrap();
        let segment = alloc_upcall(layout)
            .map(|start| {
                Segment::from_unused(start..start + nframes * PAGE_SIZE, metadata_fn).unwrap()
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

/// The trait for the global frame allocator.
///
/// OSTD allows a customized frame allocator by the [`global_frame_allocator`]
/// attribute, which marks a static variable of this type.
///
/// The API mimics the standard Rust allocator API ([`GlobalAlloc`] and
/// [`global_allocator`]). However, this trait is much safer. Double free
/// or freeing in-use memory through this trait only mess up the allocator's
/// state rather than causing undefined behavior.
///
/// Whenever OSTD or other modules need to allocate or deallocate frames via
/// [`FrameAllocOptions`], they are forwarded to the global frame allocator.
/// It is not encoraged to call the global allocator directly.
///
/// [`global_frame_allocator`]: crate::global_frame_allocator
/// [`GlobalAlloc`]: core::alloc::GlobalAlloc
pub trait GlobalFrameAllocator: Sync {
    /// Allocates a contiguous range of frames.
    fn alloc(&self, layout: Layout) -> Option<Paddr>;

    /// Deallocates a contiguous range of frames.
    fn dealloc(&self, addr: Paddr, size: usize);
}

extern "Rust" {
    /// The global frame allocator's reference exported by
    /// [`crate::global_frame_allocator`].
    static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn GlobalFrameAllocator;
}

/// Directly allocates a contiguous range of frames.
fn alloc_upcall(layout: core::alloc::Layout) -> Option<Paddr> {
    // SAFETY: We believe that the global frame allocator is set up correctly
    // with the `global_frame_allocator` attribute. If they use safe code only
    // then the up-call is safe.
    unsafe { __GLOBAL_FRAME_ALLOCATOR_REF.alloc(layout) }
}

/// Up-call to add a range of frames to the global frame allocator.
///
/// It would return the frame to the allocator for further use. This would like
/// to be done after the release of the metadata to avoid re-allocation before
/// the metadata is reset.
pub(super) fn dealloc_upcall(addr: Paddr, size: usize) {
    // SAFETY: We believe that the global frame allocator is set up correctly
    // with the `global_frame_allocator` attribute. If they use safe code only
    // then the up-call is safe.
    unsafe { __GLOBAL_FRAME_ALLOCATOR_REF.dealloc(addr, size) }
}

static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Returns whether the global frame allocator is initialized.
pub(in crate::mm) fn is_initialized() -> bool {
    IS_INITIALIZED.load(Ordering::Relaxed)
}

/// Initializes the global frame allocator.
///
/// It just does adds the frames to the global frame allocator. Calling it
/// multiple times would be not safe.
///
/// # Safety
///
/// This function should be called only once.
pub(crate) unsafe fn init() {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;

    let early_allocated_regions = EARLY_ALLOCATOR.lock().as_ref().unwrap().allocated_regions();

    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned, and skip if it is too small.
            let start = region.base().align_up(PAGE_SIZE);
            let region_end = region.base().checked_add(region.len()).unwrap();
            let end = region_end.align_down(PAGE_SIZE);
            if end <= start {
                continue;
            }

            // Add global free pages to the frame allocator.
            // Truncate the early allocated frames if there is an overlap.
            let mut added = start..end;
            if (start..end).contains(&early_allocated_regions.0.start) {
                let new_start = early_allocated_regions.0.end;
                if new_start < end {
                    added = new_start..end;
                } else {
                    continue;
                }
            }
            if let Some(max_region) = early_allocated_regions.1.as_ref() {
                if (start..end).contains(&max_region.start) {
                    let new_start = max_region.end;
                    if new_start < end {
                        added = new_start..end;
                    } else {
                        continue;
                    }
                }
            }

            log::info!(
                "Adding free frames to the allocator: 0x{:x}-0x{:x}",
                added.start,
                added.end
            );

            dealloc_upcall(added.start, added.len());
        }
    }
    IS_INITIALIZED.store(true, Ordering::Relaxed);
}

/// An allocator in the early boot phase when frame metadata is not available.
struct EarlyFrameAllocator {
    under_4g_region: MemoryRegion,
    under_4g_end: Paddr,

    max_region: Option<MemoryRegion>,
    max_end: Option<Paddr>,
}

static EARLY_ALLOCATOR: spin::Mutex<Option<EarlyFrameAllocator>> = spin::Mutex::new(None);

impl EarlyFrameAllocator {
    /// Creates a new early frame allocator.
    ///
    /// It uses at most 2 regions, the first is the maximum usable region below
    /// 4 GiB. The other is the maximum usable region above 4 GiB and is only
    /// usable when linear mapping is constructed.
    pub fn new() -> Self {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        let mut max_u4g_size = 0;
        let mut max_u4g_index = 0;

        let mut max_size = 0;
        let mut max_index = 0;
        for (i, region) in regions.iter().enumerate() {
            if region.typ() != MemoryRegionType::Usable {
                continue;
            }
            const PADDR4G: Paddr = 0x1_0000_0000;
            if region.base() < PADDR4G && region.len() > max_u4g_size {
                max_u4g_size = region.len();
                max_u4g_index = i;
            }
            if region.base() >= PADDR4G && region.len() > max_size {
                max_size = region.len();
                max_index = i;
            }
        }
        let under_4g_region = regions[max_u4g_index];
        let max_region = (max_index != 0).then(|| regions[max_index]);
        log::debug!(
            "Early frame allocator (below 4G) at: {:#x}-{:#x}",
            under_4g_region.base(),
            under_4g_region.end()
        );
        if max_region.is_some() {
            log::debug!(
                "Early frame allocator (above 4G) at: {:#x}-{:#x}",
                max_region.as_ref().unwrap().base(),
                max_region.as_ref().unwrap().end()
            );
        }
        let under_4g_end = under_4g_region.base().align_up(PAGE_SIZE);
        let max_end = max_region
            .as_ref()
            .map(|region| region.base().align_up(PAGE_SIZE));
        Self {
            under_4g_region,
            under_4g_end,
            max_region,
            max_end,
        }
    }

    /// Allocates a contiguous range of frames.
    pub fn alloc(&mut self, layout: Layout) -> Option<Paddr> {
        let size = layout.size().align_up(PAGE_SIZE);
        let allocated = self.under_4g_end.align_up(layout.align());
        if allocated + size <= self.under_4g_region.end() {
            // Allocated below 4G.
            self.under_4g_end = allocated + size;
            Some(allocated)
        } else {
            // Try above 4G.
            let max_end = self.max_end.as_mut()?;
            let allocated = max_end.align_up(layout.align());
            if allocated + size > self.max_region.as_ref().unwrap().end() {
                return None;
            }
            *max_end = allocated + size;
            Some(allocated)
        }
    }

    fn allocated_regions(&self) -> (Range<Paddr>, Option<Range<Paddr>>) {
        (
            self.under_4g_region.base().align_up(PAGE_SIZE)..self.under_4g_end,
            self.max_region
                .as_ref()
                .map(|region| region.base().align_up(PAGE_SIZE)..self.max_end.unwrap()),
        )
    }
}

/// Allocates a contiguous range of frames in the early boot phase.
pub(crate) fn early_alloc(layout: Layout) -> Option<Paddr> {
    let mut early_allocator = EARLY_ALLOCATOR.lock();
    if early_allocator.is_none() {
        *early_allocator = Some(EarlyFrameAllocator::new());
    }
    early_allocator.as_mut().unwrap().alloc(layout)
}
