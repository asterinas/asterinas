// SPDX-License-Identifier: MPL-2.0

//! Physical memory page management.
//!
//! A page is an aligned, contiguous range of bytes in physical memory. The sizes
//! of base pages and huge pages are architecture-dependent. A page can be mapped
//! to a virtual address using the page table.
//!
//! Pages can be accessed through page handles, namely, [`Page`]. A page handle
//! is a reference-counted handle to a page. When all handles to a page are dropped,
//! the page is released and can be reused.
//!
//! Pages can have dedicated metadata, which is implemented in the [`meta`] module.
//! The reference count and usage of a page are stored in the metadata as well, leaving
//! the handle only a pointer to the metadata.

pub mod allocator;
mod cont_pages;
pub mod meta;

use core::{
    any::Any,
    marker::PhantomData,
    mem::ManuallyDrop,
    sync::atomic::{AtomicUsize, Ordering},
};

pub use cont_pages::ContPages;
use meta::{
    mapping, MetaSlot, PageMeta, PAGE_METADATA_MAX_ALIGN, PAGE_METADATA_MAX_SIZE, REF_COUNT_UNUSED,
};

use super::{frame::FrameMeta, Frame, PagingLevel, PAGE_SIZE};
use crate::mm::{Paddr, PagingConsts, Vaddr};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// A page with a statically-known usage, whose metadata is represented by `M`.
#[derive(Debug)]
pub struct Page<M: PageMeta> {
    pub(super) ptr: *const MetaSlot,
    pub(super) _marker: PhantomData<M>,
}

unsafe impl<M: PageMeta> Send for Page<M> {}

unsafe impl<M: PageMeta> Sync for Page<M> {}

impl<M: PageMeta> Page<M> {
    /// Get a `Page` handle with a specific usage from a raw, unused page.
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

        // Checking unsafe preconditions of the `PageMeta` trait.
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
            .expect("Page already in use when trying to get a new handle");

        // SAFETY: We have exclusive access to the page metadata.
        let vtable_ptr = unsafe { &mut *slot.vtable_ptr.get() };
        vtable_ptr.write(core::ptr::metadata(&metadata as &dyn PageMeta));

        // SAFETY:
        // 1. `ptr` points to the first field of `MetaSlot` (guaranteed by `repr(C)`), which is the
        //    metadata storage.
        // 2. The size and the alignment of the metadata storage is large enough to hold `M`
        //    (guaranteed by the safety requirement of the `PageMeta` trait).
        // 3. We have exclusive access to the metadata storage (guaranteed by the reference count).
        unsafe { ptr.cast::<M>().cast_mut().write(metadata) };

        // Assuming no one can create a `Page` instance directly from the page address, `Relaxed`
        // is fine here. Otherwise, we should use `Release` to ensure that the metadata
        // initialization won't be reordered after this memory store.
        slot.ref_count.store(1, Ordering::Relaxed);

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Forget the handle to the page.
    ///
    /// This will result in the page being leaked without calling the custom dropper.
    ///
    /// A physical address to the page is returned in case the page needs to be
    /// restored using [`Page::from_raw`] later. This is useful when some architectural
    /// data structures need to hold the page handle such as the page table.
    #[allow(unused)]
    pub(in crate::mm) fn into_raw(self) -> Paddr {
        let paddr = self.paddr();
        core::mem::forget(self);
        paddr
    }

    /// Restore a forgotten `Page` from a physical address.
    ///
    /// # Safety
    ///
    /// The caller should only restore a `Page` that was previously forgotten using
    /// [`Page::into_raw`].
    ///
    /// And the restoring operation should only be done once for a forgotten
    /// `Page`. Otherwise double-free will happen.
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

    /// Get the metadata of this page.
    pub fn meta(&self) -> &M {
        // SAFETY: `self.ptr` points to the metadata storage which is valid to be immutably
        // borrowed as `M` because the type is correct, it lives under the given lifetime, and no
        // one will mutably borrow the page metadata after initialization.
        unsafe { &*self.ptr.cast() }
    }

    /// Get the reference count of the page.
    ///
    /// It returns the number of all references to the page, including all the
    /// existing page handles ([`Page`], [`DynPage`]), and all the mappings in the
    /// page table that points to the page.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.slot().ref_count.load(Ordering::Relaxed)
    }

    fn slot(&self) -> &MetaSlot {
        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking an
        // immutable reference to it is always safe.
        unsafe { &*self.ptr }
    }
}

impl<M: PageMeta> Clone for Page<M> {
    fn clone(&self) -> Self {
        self.slot().ref_count.fetch_add(1, Ordering::Relaxed);
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: PageMeta> Drop for Page<M> {
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

/// A page with a dynamically-known usage.
///
/// It can also be used when the user don't care about the usage of the page.
#[derive(Debug)]
pub struct DynPage {
    ptr: *const MetaSlot,
}

unsafe impl Send for DynPage {}
unsafe impl Sync for DynPage {}

impl DynPage {
    /// Forget the handle to the page.
    ///
    /// This is the same as [`Page::into_raw`].
    ///
    /// This will result in the page being leaked without calling the custom dropper.
    ///
    /// A physical address to the page is returned in case the page needs to be
    /// restored using [`Self::from_raw`] later.
    pub(in crate::mm) fn into_raw(self) -> Paddr {
        let paddr = self.paddr();
        core::mem::forget(self);
        paddr
    }

    /// Restore a forgotten page from a physical address.
    ///
    /// # Safety
    ///
    /// The safety concerns are the same as [`Page::from_raw`].
    pub(in crate::mm) unsafe fn from_raw(paddr: Paddr) -> Self {
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self { ptr }
    }

    /// Get the metadata of this page.
    pub fn meta(&self) -> &dyn Any {
        let slot = self.slot();

        // SAFETY: The page metadata is valid to be borrowed immutably, since it will never be
        // borrowed mutably after initialization.
        let vtable_ptr = unsafe { &*slot.vtable_ptr.get() };

        // SAFETY: The page metadata is initialized and valid.
        let vtable_ptr = *unsafe { vtable_ptr.assume_init_ref() };

        let meta_ptr: *const dyn PageMeta = core::ptr::from_raw_parts(self.ptr, vtable_ptr);

        // SAFETY: `self.ptr` points to the metadata storage which is valid to be immutably
        // borrowed under `vtable_ptr` because the vtable is correct, it lives under the given
        // lifetime, and no one will mutably borrow the page metadata after initialization.
        (unsafe { &*meta_ptr }) as &dyn Any
    }

    /// Get the physical address of the start of the page
    pub fn paddr(&self) -> Paddr {
        mapping::meta_to_page::<PagingConsts>(self.ptr as Vaddr)
    }

    /// Get the paging level of this page.
    pub fn level(&self) -> PagingLevel {
        1
    }

    /// Size of this page in bytes.
    pub fn size(&self) -> usize {
        PAGE_SIZE
    }

    fn slot(&self) -> &MetaSlot {
        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking an
        // immutable reference to it is always safe.
        unsafe { &*self.ptr }
    }
}

impl<M: PageMeta> TryFrom<DynPage> for Page<M> {
    type Error = DynPage;

    /// Try converting a [`DynPage`] into the statically-typed [`Page`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(dyn_page: DynPage) -> Result<Self, Self::Error> {
        if dyn_page.meta().is::<M>() {
            let result = Page {
                ptr: dyn_page.ptr,
                _marker: PhantomData,
            };
            let _ = ManuallyDrop::new(dyn_page);
            Ok(result)
        } else {
            Err(dyn_page)
        }
    }
}

impl<M: PageMeta> From<Page<M>> for DynPage {
    fn from(page: Page<M>) -> Self {
        let result = Self { ptr: page.ptr };
        let _ = ManuallyDrop::new(page);
        result
    }
}

impl From<Frame> for DynPage {
    fn from(frame: Frame) -> Self {
        Page::<FrameMeta>::from(frame).into()
    }
}

impl Clone for DynPage {
    fn clone(&self) -> Self {
        self.slot().ref_count.fetch_add(1, Ordering::Relaxed);
        Self { ptr: self.ptr }
    }
}

impl Drop for DynPage {
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

    let old = slot.ref_count.fetch_add(1, Ordering::Relaxed);
    debug_assert!(old > 0);
}
