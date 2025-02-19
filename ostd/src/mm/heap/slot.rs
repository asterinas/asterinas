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

/// A slot going to become or turned from a heap allocation.
///
/// Heap slots can come from [`Slab`] or directly from a typed page.
///
/// Since [`Slab`] can't allocate slots larger than [`PAGE_SIZE`], if the
/// size is larger than [`PAGE_SIZE`], the slot will be directly from
/// a typed [`Segment`], otherwise it will be from a [`Slab`].
///
/// Heap slots can be used to fulfill heap allocations requested by [`alloc`].
/// Upon deallocation, the deallocated memory also becomes a heap slot.
///
/// [`Slab`]: super::Slab
pub struct HeapSlot {
    /// The address of the slot.
    addr: NonNull<u8>,
    /// The size of the slot.
    size: usize,
}

impl HeapSlot {
    /// Creates a new heap slot.
    ///
    /// A slot should be valid as a return value of [`GlobalAlloc::alloc`].
    ///
    /// # Safety
    ///
    /// The pointer to the slot must be a free slot that:
    ///  - is not pointed to by any other [`HeapSlot`]s;
    ///  - and is valid for an allocation of `size` bytes.
    ///
    /// By "valid for an allocation" it means that the pointer
    ///  1. must point to a virtual address that is linearly mapped in a
    ///     physical page used for kernel heap, therefore not aliasing with
    ///     other memory and protected from the users or devices;
    ///  2. must be free of use, i.e. not allocated for other objects.
    ///
    /// [`GlobalAlloc::alloc`]: core::alloc::GlobalAlloc::alloc
    pub(super) unsafe fn new(addr: NonNull<u8>, size: usize) -> Self {
        Self { addr, size }
    }

    /// Allocates a large slot.
    ///
    /// # Panics
    ///
    /// Panics if `size` is not larger than [`PAGE_SIZE`].
    pub fn alloc_large(size: usize) -> Result<Self, AllocError> {
        assert!(size > PAGE_SIZE);

        let nframes = size.div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(nframes, |_| LargeAllocFrameMeta)
            .map_err(|_| {
                log::error!("Failed to allocate a large slot");
                AllocError
            })?;

        let paddr_range = segment.into_raw();
        let vaddr = paddr_to_vaddr(paddr_range.start);

        Ok(unsafe { Self::new(NonNull::new(vaddr as *mut u8).unwrap(), size) })
    }

    /// Deallocates a large slot.
    ///
    /// # Panics
    ///
    /// Panics if the size is not larger than [`PAGE_SIZE`].
    pub fn dealloc_large(self) {
        assert!(self.size > PAGE_SIZE);
        let nframes = self.size.div_ceil(PAGE_SIZE);
        let range = self.paddr()..self.paddr() + nframes;

        // SAFETY: The segment was once forgotten when allocated.
        drop(unsafe { Segment::<LargeAllocFrameMeta>::from_raw(range) });
    }

    /// Gets the physical address of the slot.
    pub fn paddr(&self) -> Paddr {
        self.addr.as_ptr() as Vaddr - LINEAR_MAPPING_BASE_VADDR
    }

    /// Gets the size of the slot.
    pub fn size(&self) -> usize {
        self.size
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
