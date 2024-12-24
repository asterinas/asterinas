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
mod segment;
pub mod untyped;

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

use meta::{mapping, FrameMeta, MetaSlot, PAGE_METADATA_MAX_ALIGN, PAGE_METADATA_MAX_SIZE};
pub use segment::Segment;
use untyped::{UntypedFrame, UntypedMeta};

use crate::mm::{Paddr, PagingConsts, PagingLevel, Vaddr, PAGE_SIZE};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// A page with a statically-known usage, whose metadata is represented by `M`.
#[derive(Debug)]
#[repr(transparent)]
pub struct Frame<M: FrameMeta + ?Sized> {
    pub(super) ptr: *const MetaSlot,
    pub(super) _marker: PhantomData<M>,
}

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

        // SAFETY: The aligned pointer points to a initialized `MetaSlot`.
        let ref_count = unsafe { &(*ptr).ref_count };

        ref_count
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .expect("Frame already in use when trying to get a new handle");

        // SAFETY: The aligned pointer points to a initialized `MetaSlot`.
        let vtable_ptr = unsafe { (*ptr).vtable_ptr.get() };

        // SAFETY: The pointer is valid and we have the exclusive access.
        unsafe { vtable_ptr.write(core::ptr::metadata(&metadata as &dyn FrameMeta)) };

        // Initialize the metadata
        // SAFETY: The pointer points to the first byte of the `MetaSlot`
        // structure, and layout ensured enough space for `M`. The original
        // value does not represent any object that's needed to be dropped.
        unsafe { (ptr as *mut M).write(metadata) };

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Get the metadata of this page.
    pub fn meta(&self) -> &M {
        // SAFETY: The pointer is valid and the type is correct.
        unsafe { &*(self.ptr as *const M) }
    }
}

impl<M: FrameMeta + ?Sized> Frame<M> {
    /// Get the physical address.
    pub fn paddr(&self) -> Paddr {
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
        // SAFETY: The pointer is valid and no other writes will be done to it.
        let vtable_ptr = unsafe { *(*self.ptr).vtable_ptr.get() };

        let meta_ptr: *const dyn FrameMeta = core::ptr::from_raw_parts(self.ptr, vtable_ptr);

        // SAFETY: The pointer is valid and the type is correct for the stored
        // metadata.
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
        let paddr = self.paddr();
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
}

impl<M: FrameMeta + ?Sized> Clone for Frame<M> {
    fn clone(&self) -> Self {
        self.ref_count().fetch_add(1, Ordering::Relaxed);
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: FrameMeta + ?Sized> Drop for Frame<M> {
    fn drop(&mut self) {
        let last_ref_cnt = self.ref_count().fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt > 0);
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
            let result = Frame {
                ptr: dyn_frame.ptr,
                _marker: PhantomData,
            };
            let _ = ManuallyDrop::new(dyn_frame);
            Ok(result)
        } else {
            Err(dyn_frame)
        }
    }
}

impl<M: FrameMeta> From<Frame<M>> for Frame<dyn FrameMeta> {
    fn from(frame: Frame<M>) -> Self {
        let result = Self {
            ptr: frame.ptr,
            _marker: PhantomData,
        };
        let _ = ManuallyDrop::new(frame);
        result
    }
}

impl From<UntypedFrame> for Frame<dyn FrameMeta> {
    fn from(frame: UntypedFrame) -> Self {
        Frame::<UntypedMeta>::from(frame).into()
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
    // SAFETY: The virtual address points to an initialized metadata slot.
    let slot = unsafe { &*(vaddr as *const MetaSlot) };
    let old = slot.ref_count.fetch_add(1, Ordering::Relaxed);

    debug_assert!(old > 0);
}
