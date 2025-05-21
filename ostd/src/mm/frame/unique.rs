// SPDX-License-Identifier: MPL-2.0

//! The unique frame pointer that is not shared with others.

use core::{marker::PhantomData, mem::ManuallyDrop, sync::atomic::Ordering};

use super::{
    meta::{GetFrameError, REF_COUNT_UNIQUE},
    AnyFrameMeta, Frame, MetaSlot,
};
use crate::mm::{frame::mapping, Paddr, PagingConsts, PagingLevel, PAGE_SIZE};

/// An owning frame pointer.
///
/// Unlike [`Frame`], the frame pointed to by this pointer is not shared with
/// others. So a mutable reference to the metadata is available for the frame.
#[repr(transparent)]
pub struct UniqueFrame<M: AnyFrameMeta + ?Sized> {
    ptr: *const MetaSlot,
    _marker: PhantomData<M>,
}

unsafe impl<M: AnyFrameMeta + ?Sized> Send for UniqueFrame<M> {}

unsafe impl<M: AnyFrameMeta + ?Sized> Sync for UniqueFrame<M> {}

impl<M: AnyFrameMeta + ?Sized> core::fmt::Debug for UniqueFrame<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UniqueFrame({:#x})", self.start_paddr())
    }
}

impl<M: AnyFrameMeta> UniqueFrame<M> {
    /// Gets a [`UniqueFrame`] with a specific usage from a raw, unused page.
    ///
    /// The caller should provide the initial metadata of the page.
    pub fn from_unused(paddr: Paddr, metadata: M) -> Result<Self, GetFrameError> {
        Ok(Self {
            ptr: MetaSlot::get_from_unused(paddr, metadata, true)?,
            _marker: PhantomData,
        })
    }

    /// Repurposes the frame with a new metadata.
    pub fn repurpose<M1: AnyFrameMeta>(self, metadata: M1) -> UniqueFrame<M1> {
        // SAFETY: We are the sole owner and the metadata is initialized.
        unsafe { self.slot().drop_meta_in_place() };
        // SAFETY: We are the sole owner.
        unsafe { self.slot().write_meta(metadata) };
        // SAFETY: The metadata is initialized with type `M1`.
        unsafe { core::mem::transmute(self) }
    }

    /// Gets the metadata of this page.
    pub fn meta(&self) -> &M {
        // SAFETY: The type is tracked by the type system.
        unsafe { &*self.slot().as_meta_ptr::<M>() }
    }

    /// Gets the mutable metadata of this page.
    pub fn meta_mut(&mut self) -> &mut M {
        // SAFETY: The type is tracked by the type system.
        // And we have the exclusive access to the metadata.
        unsafe { &mut *self.slot().as_meta_ptr::<M>() }
    }
}

impl<M: AnyFrameMeta + ?Sized> UniqueFrame<M> {
    /// Gets the physical address of the start of the frame.
    pub fn start_paddr(&self) -> Paddr {
        self.slot().frame_paddr()
    }

    /// Gets the paging level of this page.
    ///
    /// This is the level of the page table entry that maps the frame,
    /// which determines the size of the frame.
    ///
    /// Currently, the level is always 1, which means the frame is a regular
    /// page frame.
    pub const fn level(&self) -> PagingLevel {
        1
    }

    /// Gets the size of this page in bytes.
    pub const fn size(&self) -> usize {
        PAGE_SIZE
    }

    /// Gets the dyncamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta(&self) -> &dyn AnyFrameMeta {
        // SAFETY: The metadata is initialized and valid.
        unsafe { &*self.slot().dyn_meta_ptr() }
    }

    /// Gets the dyncamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta_mut(&mut self) -> &mut dyn AnyFrameMeta {
        // SAFETY: The metadata is initialized and valid. We have the exclusive
        // access to the frame.
        unsafe { &mut *self.slot().dyn_meta_ptr() }
    }

    /// Resets the frame to unused without up-calling the allocator.
    ///
    /// This is solely useful for the allocator implementation/testing and
    /// is highly experimental. Usage of this function is discouraged.
    ///
    /// Usage of this function other than the allocator would actually leak
    /// the frame since the allocator would not be aware of the frame.
    //
    // FIXME: We may have a better `Segment` and `UniqueSegment` design to
    // allow the allocator hold the ownership of all the frames in a chunk
    // instead of the head. Then this weird public API can be `#[cfg(ktest)]`.
    pub fn reset_as_unused(self) {
        let this = ManuallyDrop::new(self);

        this.slot().ref_count.store(0, Ordering::Release);
        // SAFETY: We are the sole owner and the reference count is 0.
        // The slot is initialized.
        unsafe { this.slot().drop_last_in_place() };
    }

    /// Converts this frame into a raw physical address.
    pub(crate) fn into_raw(self) -> Paddr {
        let this = ManuallyDrop::new(self);
        this.start_paddr()
    }

    /// Restores a raw physical address back into a unique frame.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and points to
    /// a forgotten frame that was previously casted by [`Self::into_raw`].
    pub(crate) unsafe fn from_raw(paddr: Paddr) -> Self {
        let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    pub(super) fn slot(&self) -> &MetaSlot {
        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be
        // mutably borrowed, so taking an immutable reference to it is safe.
        unsafe { &*self.ptr }
    }
}

impl<M: AnyFrameMeta + ?Sized> Drop for UniqueFrame<M> {
    fn drop(&mut self) {
        self.slot().ref_count.store(0, Ordering::Relaxed);
        // SAFETY: We are the sole owner and the reference count is 0.
        // The slot is initialized.
        unsafe { self.slot().drop_last_in_place() };

        super::allocator::get_global_frame_allocator().dealloc(self.start_paddr(), PAGE_SIZE);
    }
}

impl<M: AnyFrameMeta + ?Sized> From<UniqueFrame<M>> for Frame<M> {
    fn from(unique: UniqueFrame<M>) -> Self {
        // The `Release` ordering make sure that previous writes are visible
        // before the reference count is set to 1. It pairs with
        // `MetaSlot::get_from_in_use`.
        unique.slot().ref_count.store(1, Ordering::Release);
        // SAFETY: The internal representation is now the same.
        unsafe { core::mem::transmute(unique) }
    }
}

impl<M: AnyFrameMeta + ?Sized> TryFrom<Frame<M>> for UniqueFrame<M> {
    type Error = Frame<M>;

    /// Tries to get a unique frame from a shared frame.
    ///
    /// If the reference count is not 1, the frame is returned back.
    fn try_from(frame: Frame<M>) -> Result<Self, Self::Error> {
        match frame.slot().ref_count.compare_exchange(
            1,
            REF_COUNT_UNIQUE,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                // SAFETY: The reference count is now `REF_COUNT_UNIQUE`.
                Ok(unsafe { core::mem::transmute::<Frame<M>, UniqueFrame<M>>(frame) })
            }
            Err(_) => Err(frame),
        }
    }
}
