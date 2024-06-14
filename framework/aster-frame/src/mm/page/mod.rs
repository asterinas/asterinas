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
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

use meta::{mapping, MetaSlot, PageMeta};

use super::PAGE_SIZE;
use crate::mm::{Paddr, PagingConsts, Vaddr};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// Representing a page that has a statically-known usage purpose,
/// whose metadata is represented by `M`.
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
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is out of bound or not aligned;
    ///  - the page is already in use.
    pub fn from_unused(paddr: Paddr) -> Self {
        assert!(paddr % PAGE_SIZE == 0);
        assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        let usage = unsafe { &(*ptr).usage };
        let get_ref_count = unsafe { &(*ptr).ref_count };

        usage
            .compare_exchange(0, M::USAGE as u8, Ordering::SeqCst, Ordering::Relaxed)
            .expect("page already in use when trying to get a new handle");

        let old_get_ref_count = get_ref_count.fetch_add(1, Ordering::Relaxed);
        debug_assert!(old_get_ref_count == 0);

        // Initialize the metadata
        // SAFETY: The pointer points to the first byte of the `MetaSlot`
        // structure, and layout ensured enoungh space for `M`. The original
        // value does not represent any object that's needed to be dropped.
        unsafe { (ptr as *mut M).write(M::default()) };

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

    /// Get the metadata of this page.
    pub fn meta(&self) -> &M {
        unsafe { &*(self.ptr as *const M) }
    }

    /// Get the mutable metadata of this page.
    ///
    /// # Safety
    ///
    /// The caller should be sure that the page is exclusively owned.
    pub(in crate::mm) unsafe fn meta_mut(&mut self) -> &mut M {
        unsafe { &mut *(self.ptr as *mut M) }
    }

    fn get_ref_count(&self) -> &AtomicU32 {
        unsafe { &(*self.ptr).ref_count }
    }
}

impl<M: PageMeta> Clone for Page<M> {
    fn clone(&self) -> Self {
        self.get_ref_count().fetch_add(1, Ordering::Relaxed);
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: PageMeta> Drop for Page<M> {
    fn drop(&mut self) {
        if self.get_ref_count().fetch_sub(1, Ordering::Release) == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);
            // Let the custom dropper handle the drop.
            M::on_drop(self);
            // Drop the metadata.
            unsafe {
                core::ptr::drop_in_place(self.ptr as *mut M);
            }
            // No handles means no usage. This also releases the page as unused for further
            // calls to `Page::from_unused`.
            unsafe { &*self.ptr }.usage.store(0, Ordering::Release);
            // Deallocate the page.
            // It would return the page to the allocator for further use. This would be done
            // after the release of the metadata to avoid re-allocation before the metadata
            // is reset.
            allocator::PAGE_ALLOCATOR
                .get()
                .unwrap()
                .lock()
                .dealloc(self.paddr() / PAGE_SIZE, 1);
        };
    }
}
