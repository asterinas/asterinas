// SPDX-License-Identifier: MPL-2.0

//! Physical memory page management.
//!
//! A page is an aligned, contiguous range of bytes in physical memory. The sizes
//! of base pages and huge pages are architecture-dependent. A page can be mapped
//! to a virtual address using the page table.
//!
//! Pages can be accessed through page handles, namely, [`Frame`]. A page handle
//! is a reference-counted handle to a page. When all handles to a page are dropped,
//! the page is released and can be reused.
//!
//! Pages can have dedicated metadata, which is implemented in the [`meta`] module.
//! The reference count and usage of a page are stored in the metadata as well, leaving
//! the handle only a pointer to the metadata.

pub mod allocator;
pub mod meta;
pub mod segment;
pub mod untyped;

use core::{
    marker::PhantomData,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

use meta::{
    mapping, FrameMeta, MetaSlot, PAGE_METADATA_MAX_ALIGN, PAGE_METADATA_MAX_SIZE, REF_COUNT_UNUSED,
};
pub use segment::Segment;
use untyped::{DynUFrame, UFrameMeta};

use super::{PagingLevel, PAGE_SIZE};
use crate::mm::{Paddr, PagingConsts, Vaddr};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// A physical memory frame with a statically-known usage, whose metadata is represented by `M`.
#[derive(Debug)]
#[repr(transparent)]
pub struct Frame<M: FrameMeta + ?Sized> {
    pub(super) ptr: *const MetaSlot,
    pub(super) _marker: PhantomData<M>,
}

/// A physical memory frame with a dynamically-known usage.
///
/// The usage of this frame will not be changed while this object is alive. But the
/// usage is not known at compile time. An [`DynFrame`] as a parameter accepts any
/// type of frames.
pub type DynFrame = Frame<dyn FrameMeta>;

unsafe impl<M: FrameMeta + ?Sized> Send for Frame<M> {}

unsafe impl<M: FrameMeta + ?Sized> Sync for Frame<M> {}

impl<M: FrameMeta> Frame<M> {
    /// Get a `Frame` handle with a specific usage from a raw, unused page.
    ///
    /// The caller should provide the initial metadata of the page.
    ///
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is out of bound or not aligned;
    ///  - the page is already in use.
    pub fn from_unused(paddr: Paddr, metadata: M) -> Self {
        assert!(paddr % PAGE_SIZE == 0);
        assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);

        // Checking unsafe preconditions of the `FrameMeta` trait.
        debug_assert!(size_of::<M>() <= PAGE_METADATA_MAX_SIZE);
        debug_assert!(align_of::<M>() <= PAGE_METADATA_MAX_ALIGN);

        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking an
        // immutable reference to it is always safe.
        let slot = unsafe { &*ptr };

        // `Acquire` pairs with the `Release` in `drop_last_in_place` and ensures the metadata
        // initialization won't be reordered before this memory compare-and-exchange.
        slot.ref_count
            .compare_exchange(REF_COUNT_UNUSED, 0, Ordering::Acquire, Ordering::Relaxed)
            .expect("Frame already in use when trying to get a new handle");

        // SAFETY: We have exclusive access to the page metadata. These fields are mutably
        // borrowed only once.
        let vtable_ptr = unsafe { &mut *slot.vtable_ptr.get() };
        vtable_ptr.write(core::ptr::metadata(&metadata as &dyn FrameMeta));

        // SAFETY:
        // 1. `ptr` points to the first field of `MetaSlot` (guaranteed by `repr(C)`), which is the
        //    metadata storage.
        // 2. The size and the alignment of the metadata storage is large enough to hold `M`
        //    (guaranteed by the safety requirement of the `FrameMeta` trait).
        // 3. We have exclusive access to the metadata storage (guaranteed by the reference count).
        unsafe { ptr.cast::<M>().cast_mut().write(metadata) };

        // Assuming no one can create a `Frame` instance directly from the page address, `Relaxed`
        // is fine here. Otherwise, we should use `Release` to ensure that the metadata
        // initialization won't be reordered after this memory store.
        slot.ref_count.store(1, Ordering::Relaxed);

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Get the metadata of this page.
    pub fn meta(&self) -> &M {
        // SAFETY: `self.ptr` points to the metadata storage which is valid to
        // be immutably borrowed as `M` because the type is correct, it lives
        // under the given lifetime, and no one will mutably borrow the page
        // metadata after initialization.
        unsafe { &*self.ptr.cast() }
    }
}

impl<M: FrameMeta + ?Sized> Frame<M> {
    /// Get the physical address.
    pub fn start_paddr(&self) -> Paddr {
        mapping::meta_to_page::<PagingConsts>(self.ptr as Vaddr)
    }

    /// Get the paging level of this page.
    ///
    /// This is the level of the page table entry that maps the frame,
    /// which determines the size of the frame.
    ///
    /// Currently, the level is always 1, which means the frame is a regular
    /// page frame.
    pub const fn level(&self) -> PagingLevel {
        1
    }

    /// Size of this page in bytes.
    pub const fn size(&self) -> usize {
        PAGE_SIZE
    }

    /// Get the dyncamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta(&self) -> &dyn FrameMeta {
        let slot = self.slot();

        // SAFETY: The page metadata is valid to be borrowed immutably, since it will never be
        // borrowed mutably after initialization.
        let vtable_ptr = unsafe { &*slot.vtable_ptr.get() };

        // SAFETY: The page metadata is initialized and valid.
        let vtable_ptr = *unsafe { vtable_ptr.assume_init_ref() };

        let meta_ptr: *const dyn FrameMeta = core::ptr::from_raw_parts(self.ptr, vtable_ptr);

        // SAFETY: `self.ptr` points to the metadata storage which is valid to be immutably
        // borrowed under `vtable_ptr` because the vtable is correct, it lives under the given
        // lifetime, and no one will mutably borrow the page metadata after initialization.
        unsafe { &*meta_ptr }
    }

    /// Get the reference count of the page.
    ///
    /// It returns the number of all references to the page, including all the
    /// existing page handles ([`Frame`], [`Frame<dyn FrameMeta>`]), and all the mappings in the
    /// page table that points to the page.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.ref_count().load(Ordering::Relaxed)
    }

    fn ref_count(&self) -> &AtomicU32 {
        unsafe { &(*self.ptr).ref_count }
    }

    /// Forget the handle to the page.
    ///
    /// This will result in the page being leaked without calling the custom dropper.
    ///
    /// A physical address to the page is returned in case the page needs to be
    /// restored using [`Frame::from_raw`] later. This is useful when some architectural
    /// data structures need to hold the page handle such as the page table.
    #[allow(unused)]
    pub(in crate::mm) fn into_raw(self) -> Paddr {
        let paddr = self.start_paddr();
        core::mem::forget(self);
        paddr
    }

    /// Restore a forgotten `Frame` from a physical address.
    ///
    /// # Safety
    ///
    /// The caller should only restore a `Frame` that was previously forgotten using
    /// [`Frame::into_raw`].
    ///
    /// And the restoring operation should only be done once for a forgotten
    /// `Frame`. Otherwise double-free will happen.
    ///
    /// Also, the caller ensures that the usage of the page is correct. There's
    /// no checking of the usage in this function.
    pub(in crate::mm) unsafe fn from_raw(paddr: Paddr) -> Self {
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn slot(&self) -> &MetaSlot {
        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be
        // mutably borrowed, so taking an immutable reference to it is safe.
        unsafe { &*self.ptr }
    }
}

impl<M: FrameMeta + ?Sized> Clone for Frame<M> {
    fn clone(&self) -> Self {
        // SAFETY: We have already held a reference to the page.
        unsafe { self.slot().inc_ref_count() };

        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: FrameMeta + ?Sized> Drop for Frame<M> {
    fn drop(&mut self) {
        let last_ref_cnt = self.slot().ref_count.fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt != 0 && last_ref_cnt != REF_COUNT_UNUSED);

        if last_ref_cnt == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);

            // SAFETY: this is the last reference and is about to be dropped.
            unsafe {
                meta::drop_last_in_place(self.ptr as *mut MetaSlot);
            }
        }
    }
}

impl<M: FrameMeta> TryFrom<Frame<dyn FrameMeta>> for Frame<M> {
    type Error = Frame<dyn FrameMeta>;

    /// Try converting a [`Frame<dyn FrameMeta>`] into the statically-typed [`Frame`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(dyn_frame: Frame<dyn FrameMeta>) -> Result<Self, Self::Error> {
        if (dyn_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            // SAFETY: The metadata is coerceable and the struct is transmutable.
            Ok(unsafe { core::mem::transmute::<Frame<dyn FrameMeta>, Frame<M>>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

impl<M: FrameMeta> From<Frame<M>> for Frame<dyn FrameMeta> {
    fn from(frame: Frame<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl<M: UFrameMeta> From<Frame<M>> for DynUFrame {
    fn from(frame: Frame<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl<M: UFrameMeta> From<&Frame<M>> for &DynUFrame {
    fn from(frame: &Frame<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl From<DynUFrame> for Frame<dyn FrameMeta> {
    fn from(frame: DynUFrame) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl TryFrom<Frame<dyn FrameMeta>> for DynUFrame {
    type Error = Frame<dyn FrameMeta>;

    /// Try converting a [`Frame<dyn FrameMeta>`] into [`DynUFrame`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(dyn_frame: Frame<dyn FrameMeta>) -> Result<Self, Self::Error> {
        if dyn_frame.dyn_meta().is_untyped() {
            // SAFETY: The metadata is coerceable and the struct is transmutable.
            Ok(unsafe { core::mem::transmute::<Frame<dyn FrameMeta>, DynUFrame>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

/// Increases the reference count of the page by one.
///
/// # Safety
///
/// The caller should ensure the following conditions:
///  1. The physical address must represent a valid page;
///  2. The caller must have already held a reference to the page.
pub(in crate::mm) unsafe fn inc_page_ref_count(paddr: Paddr) {
    debug_assert!(paddr % PAGE_SIZE == 0);
    debug_assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);

    let vaddr: Vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
    // SAFETY: `vaddr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking
    // an immutable reference to it is always safe.
    let slot = unsafe { &*(vaddr as *const MetaSlot) };

    // SAFETY: We have already held a reference to the page.
    unsafe { slot.inc_ref_count() };
}
