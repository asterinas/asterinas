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

pub(crate) mod allocator;
pub(in crate::mm) mod cont_pages;
pub(in crate::mm) mod meta;

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    panic,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

use meta::{mapping, MetaSlot, PageMeta, PageUsage};

use super::{
    frame::{Frame, FrameMetaExt},
    PagingLevel, PAGE_SIZE,
};
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
    /// An initial value of the metadata of the result page should be provided.
    ///
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is out of bound or not aligned;
    ///  - the page is already in use.
    pub fn from_unused(paddr: Paddr, metadata: M) -> Self {
        assert!(paddr % PAGE_SIZE == 0);
        assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        // Try to lock the usage of the page by securing the reference count.
        // SAFETY: The aligned pointer points to an initialized `MetaSlot`.
        let ref_count = unsafe { &(*ptr).ref_count };
        ref_count
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .expect("page already in use when trying to get a new handle");

        // SAFETY: The aligned pointer points to an initialized `MetaSlot`.
        let usage = unsafe { &(*ptr).usage };
        usage
            .compare_exchange(0, M::USAGE as u8, Ordering::SeqCst, Ordering::Relaxed)
            .expect("page already in use when trying to get a new handle");

        // Initialize the metadata
        // SAFETY: The pointer points to the first byte of the `MetaSlot`
        // structure, and layout ensured enoungh space for `M`. The original
        // value does not represent any object that's needed to be dropped.
        unsafe { (ptr as *mut M).write(metadata) };

        Self {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: PageMeta> Page<M> {
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

    /// Increase the reference count of the page by one.
    ///
    /// # Safety
    ///
    /// The physical address must represent a valid page and the caller must already hold one
    /// reference count.
    pub(in crate::mm) unsafe fn inc_ref_count(paddr: Paddr) {
        let page = unsafe { ManuallyDrop::new(Self::from_raw(paddr)) };
        let _page = page.clone();
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
        // SAFETY: The pointer is valid and the metadata is initialized because
        // the handle implies such a state. Also We don't peform any mutation
        // on the metadata.
        unsafe { &*(self.ptr as *const M) }
    }

    /// Get a mutable reference to the metadata of this page.
    pub fn meta_mut(&mut self) -> &mut M {
        // SAFETY: The pointer is valid and the metadata is initialized because
        // the handle implies such a state. We have a mutable reference to the
        // handle so we can return a mutable reference to the metadata.
        unsafe { &mut *(self.ptr as *mut M) }
    }

    fn ref_count(&self) -> &AtomicU32 {
        // The pointer is valid and the metadata is initialized because
        // the handle implies such a state.
        unsafe { &(*self.ptr).ref_count }
    }
}

impl<M: PageMeta> Clone for Page<M> {
    fn clone(&self) -> Self {
        self.ref_count().fetch_add(1, Ordering::Relaxed);
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: PageMeta> Drop for Page<M> {
    fn drop(&mut self) {
        let last_ref_cnt = self.ref_count().fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt > 0);
        if last_ref_cnt == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);
            // SAFETY: this is the last reference and is about to be dropped.
            unsafe {
                meta::drop_as_last::<M>(self.ptr);
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

    /// Increase the reference count of the page by one.
    ///
    /// # Safety
    ///
    /// The physical address must represent a valid page and the caller must already hold one
    /// reference count.
    pub(in crate::mm) unsafe fn inc_ref_count(paddr: Paddr) {
        let page = unsafe { ManuallyDrop::new(Self::from_raw(paddr)) };
        let _page = page.clone();
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

    /// Get the usage of the page.
    pub fn usage(&self) -> PageUsage {
        // SAFETY: structure is safely created with a pointer that points
        // to initialized [`MetaSlot`] memory.
        let usage_raw = unsafe { (*self.ptr).usage.load(Ordering::Relaxed) };
        num::FromPrimitive::from_u8(usage_raw).unwrap()
    }

    fn ref_count(&self) -> &AtomicU32 {
        unsafe { &(*self.ptr).ref_count }
    }
}

impl TryFrom<DynPage> for Frame<dyn FrameMetaExt> {
    type Error = DynPage;

    /// Try converting a [`DynPage`] into a [`Frame<dyn FrameMetaExt>`].
    ///
    /// If the usage of the page is not the same as the expected usage, it will
    /// return the dynamic page itself as is.
    fn try_from(dyn_page: DynPage) -> Result<Self, Self::Error> {
        if dyn_page.usage() == PageUsage::Frame {
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

impl Clone for DynPage {
    fn clone(&self) -> Self {
        self.ref_count().fetch_add(1, Ordering::Relaxed);
        Self { ptr: self.ptr }
    }
}

impl Drop for DynPage {
    fn drop(&mut self) {
        let last_ref_cnt = self.ref_count().fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt > 0);
        if last_ref_cnt == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);
            // Drop the page and its metadata according to its usage.
            match self.usage() {
                PageUsage::Frame => {
                    // SAFETY: it operates on a last, about to be dropped page
                    // table page handle. And the inner metadata implements
                    // [`Pod`] so it is safe to drop as [`DefaultFrameMeta`].
                    unsafe {
                        meta::drop_as_last::<meta::FrameMeta<dyn FrameMetaExt>>(self.ptr);
                    }
                }
                PageUsage::PageTable => {
                    // SAFETY: it operates on a last, about to be dropped page
                    // table page handle.
                    unsafe {
                        meta::drop_as_last::<meta::PageTablePageMeta>(self.ptr);
                    }
                }
                // The following pages don't have metadata and can't be dropped.
                PageUsage::Unused | PageUsage::Reserved | PageUsage::Kernel | PageUsage::Meta => {
                    panic!("dropping a dynamic page with usage {:?}", self.usage());
                }
            }
        }
    }
}
