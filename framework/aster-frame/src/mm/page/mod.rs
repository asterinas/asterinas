// SPDX-License-Identifier: MPL-2.0

//! Managing pages or frames.
//!
//! A page is an aligned, contiguous range of bytes in physical memory. The sizes
//! of base pages and huge pages are architecture-dependent. A page can be mapped
//! to a virtual address using the page table.
//!
//! A frame is a special page that is _untyped_ memory. It is used to store data
//! irrelevant to the integrity of the kernel. All pages mapped to the virtual
//! address space of the users are backed by frames.

pub(crate) mod allocator;
pub(in crate::mm) mod meta;
use meta::{mapping, MetaSlot, PageMeta};
mod frame;
pub use frame::{Frame, VmFrameRef};
mod vm_frame_vec;
pub use vm_frame_vec::{FrameVecIter, VmFrameVec};
mod segment;
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

pub use segment::Segment;

use super::PAGE_SIZE;
use crate::mm::{paddr_to_vaddr, Paddr, PagingConsts, Vaddr};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// Representing a page that has a statically-known usage purpose,
/// whose metadata is represented by `M`.
#[derive(Debug)]
pub struct Page<M: PageMeta> {
    ptr: *const MetaSlot,
    _marker: PhantomData<M>,
}

unsafe impl<M: PageMeta> Send for Page<M> {}
unsafe impl<M: PageMeta> Sync for Page<M> {}

/// Errors that can occur when getting a page handle.
#[derive(Debug)]
pub enum PageHandleError {
    /// The physical address is out of range.
    OutOfRange,
    /// The physical address is not aligned to the page size.
    NotAligned,
    /// The page is already in use.
    InUse,
}

impl<M: PageMeta> Page<M> {
    /// Convert an unused page to a `Page` handle for a specific usage.
    pub(in crate::mm) fn from_unused(paddr: Paddr) -> Result<Self, PageHandleError> {
        if paddr % PAGE_SIZE != 0 {
            return Err(PageHandleError::NotAligned);
        }
        if paddr > MAX_PADDR.load(Ordering::Relaxed) {
            return Err(PageHandleError::OutOfRange);
        }

        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        let usage = unsafe { &(*ptr).usage };
        let refcnt = unsafe { &(*ptr).refcnt };

        usage
            .compare_exchange(0, M::USAGE as u8, Ordering::SeqCst, Ordering::Relaxed)
            .map_err(|_| PageHandleError::InUse)?;
        refcnt.fetch_add(1, Ordering::Relaxed);

        Ok(Self {
            ptr,
            _marker: PhantomData,
        })
    }

    /// Forget the handle to the page.
    ///
    /// This will result in the page being leaked without calling the custom dropper.
    pub fn forget(self) -> Paddr {
        let paddr = self.paddr();
        core::mem::forget(self);
        paddr
    }

    /// Restore a forgotten `Page` from a physical address.
    ///
    /// # Safety
    ///
    /// The caller should only restore a `Page` that was previously forgotten using
    /// [`Page::forget`].
    ///
    /// And the restoring operation should only be done once for a forgotten
    /// `Page`. Otherwise double-free will happen.
    ///
    /// Also, the caller ensures that the usage of the page is correct. There's
    /// no checking of the usage in this function.
    pub(in crate::mm) unsafe fn restore(paddr: Paddr) -> Self {
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Clone a `Page` handle from a forgotten `Page` as a physical address.
    ///
    /// This is similar to [`Page::restore`], but it also increments the reference count
    /// and the forgotten page will be still leaked unless restored later.
    ///
    /// # Safety
    ///
    /// The safety requirements are the same as [`Page::restore`].
    pub(in crate::mm) unsafe fn clone_restore(paddr: &Paddr) -> Self {
        let vaddr = mapping::page_to_meta::<PagingConsts>(*paddr);
        let ptr = vaddr as *const MetaSlot;

        let refcnt = unsafe { &(*ptr).refcnt };
        refcnt.fetch_add(1, Ordering::Relaxed);

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Get the physical address.
    pub fn paddr(&self) -> Paddr {
        mapping::meta_to_page::<PagingConsts>(self.ptr as Vaddr)
    }

    /// Get the reference count of this page.
    ///
    /// # Safety
    ///
    /// This method by itself is safe, but using it correctly requires extra care.
    /// Another thread can change the reference count at any time, including
    /// potentially between calling this method and the action depending on the
    /// result.
    fn ref_count(&self) -> u32 {
        self.refcnt().load(Ordering::Relaxed)
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

    fn refcnt(&self) -> &AtomicU32 {
        unsafe { &(*self.ptr).refcnt }
    }
}

impl<M: PageMeta> Clone for Page<M> {
    fn clone(&self) -> Self {
        self.refcnt().fetch_add(1, Ordering::Relaxed);
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: PageMeta> Drop for Page<M> {
    fn drop(&mut self) {
        if self.refcnt().fetch_sub(1, Ordering::Release) == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);
            // Let the custom dropper handle the drop.
            M::on_drop(self);
            // No handles means no usage.
            unsafe { &*self.ptr }.usage.store(0, Ordering::Release);
        };
    }
}
