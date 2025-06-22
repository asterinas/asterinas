// SPDX-License-Identifier: MPL-2.0

//! Heap slots for allocations.

use core::{alloc::AllocError, ptr::NonNull};

use crate::{
    impl_frame_meta_for,
    mm::{
        kspace::LINEAR_MAPPING_BASE_VADDR, paddr_to_vaddr, FrameAllocOptions, Paddr, Segment,
        Vaddr, PAGE_SIZE,
    },
};

/// A slot that will become or has been turned from a heap allocation.
///
/// Heap slots can come from [`Slab`] or directly from a typed [`Segment`].
///
/// Heap slots can be used to fulfill heap allocations requested by the allocator.
/// Upon deallocation, the deallocated memory also becomes a heap slot.
///
/// The size of the heap slot must match the slot size of the [`Slab`] or the
/// size of the [`Segment`].
///
/// [`Slab`]: super::Slab
pub struct HeapSlot {
    /// The address of the slot.
    addr: NonNull<u8>,
    /// The type and size of the slot.
    info: SlotInfo,
}

/// The type and size of the heap slot that should be used for the allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotInfo {
    /// The slot is from a [`super::Slab`].
    ///
    /// The size of the slot and the corresponding slab are provided.
    /// Both values are identical.
    SlabSlot(usize),
    /// The slot is from a [`Segment`].
    ///
    /// The size of the slot and the corresponding segment are provided.
    /// Both values are identical.
    LargeSlot(usize),
}

impl SlotInfo {
    /// Gets the size of the slot.
    pub fn size(&self) -> usize {
        match self {
            Self::SlabSlot(size) => *size,
            Self::LargeSlot(size) => *size,
        }
    }
}

impl HeapSlot {
    /// Creates a new pointer to a heap slot.
    ///
    /// # Safety
    ///
    /// The pointer to the slot must either:
    ///  - be a free slot in a [`super::Slab`], or
    ///  - be a free slot in a [`Segment`].
    ///
    /// If the pointer is from a [`super::Slab`] or [`Segment`], the slot must
    /// have a size that matches the slot size of the slab or segment respectively.
    pub(super) unsafe fn new(addr: NonNull<u8>, info: SlotInfo) -> Self {
        Self { addr, info }
    }

    /// Allocates a large slot.
    ///
    /// This function allocates in units of [`PAGE_SIZE`] bytes.
    ///
    /// This function returns an error if the frame allocation fails.
    ///
    /// # Panics
    ///
    /// This function panics if the size is not a multiple of [`PAGE_SIZE`].
    pub fn alloc_large(size: usize) -> Result<Self, AllocError> {
        assert_eq!(size % PAGE_SIZE, 0);
        let nframes = size / PAGE_SIZE;
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(nframes, |_| LargeAllocFrameMeta)
            .map_err(|_| {
                log::error!("Failed to allocate a large slot");
                AllocError
            })?;

        let paddr_range = segment.into_raw();
        let vaddr = paddr_to_vaddr(paddr_range.start);

        Ok(Self {
            addr: NonNull::new(vaddr as *mut u8).unwrap(),
            info: SlotInfo::LargeSlot(size),
        })
    }

    /// Deallocates a large slot.
    ///
    /// # Panics
    ///
    /// This function aborts if the slot was not allocated with
    /// [`HeapSlot::alloc_large`], as it requires specific memory management
    /// operations that only apply to large slots.
    pub fn dealloc_large(self) {
        let SlotInfo::LargeSlot(size) = self.info else {
            log::error!(
                "Deallocating a large slot that was not allocated with `HeapSlot::alloc_large`"
            );
            crate::panic::abort();
        };

        debug_assert_eq!(size % PAGE_SIZE, 0);
        debug_assert_eq!(self.paddr() % PAGE_SIZE, 0);
        let range = self.paddr()..self.paddr() + size;

        // SAFETY: The segment was once forgotten when allocated.
        drop(unsafe { Segment::<LargeAllocFrameMeta>::from_raw(range) });
    }

    /// Gets the physical address of the slot.
    pub fn paddr(&self) -> Paddr {
        self.addr.as_ptr() as Vaddr - LINEAR_MAPPING_BASE_VADDR
    }

    /// Gets the size of the slot.
    pub fn size(&self) -> usize {
        match self.info {
            SlotInfo::SlabSlot(size) => size,
            SlotInfo::LargeSlot(size) => size,
        }
    }

    /// Gets the type and size of the slot.
    pub fn info(&self) -> SlotInfo {
        self.info
    }

    /// Gets the pointer to the slot.
    pub fn as_ptr(&self) -> *mut u8 {
        self.addr.as_ptr()
    }
}

/// The frames allocated for a large allocation.
#[derive(Debug)]
pub struct LargeAllocFrameMeta;

impl_frame_meta_for!(LargeAllocFrameMeta);
