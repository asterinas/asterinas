// SPDX-License-Identifier: MPL-2.0

//! A contiguous range of pages.

use alloc::vec::Vec;
use core::{mem::ManuallyDrop, ops::Range};

use super::{meta::PageMeta, Page};
use crate::mm::{Paddr, PAGE_SIZE};

/// A contiguous range of physical memory pages.
///
/// This is a handle to many contiguous pages. It will be more lightweight
/// than owning an array of page handles.
///
/// The ownership is acheived by the reference counting mechanism of pages.
/// When constructing a `ContPages`, the page handles are created then
/// forgotten, leaving the reference count. When dropping a it, the page
/// handles are restored and dropped, decrementing the reference count.
#[derive(Debug)]
pub struct ContPages<M: PageMeta> {
    range: Range<Paddr>,
    _marker: core::marker::PhantomData<M>,
}

impl<M: PageMeta> Drop for ContPages<M> {
    fn drop(&mut self) {
        for i in self.range.clone().step_by(PAGE_SIZE) {
            // SAFETY: for each page there would be a forgotten handle
            // when creating the `ContPages` object.
            drop(unsafe { Page::<M>::from_raw(i) });
        }
    }
}

impl<M: PageMeta> ContPages<M> {
    /// Create a new `ContPages` from unused pages.
    ///
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is invalid or not aligned;
    ///  - any of the pages are already in use.
    pub fn from_unused(range: Range<Paddr>) -> Self {
        for i in range.clone().step_by(PAGE_SIZE) {
            let _ = ManuallyDrop::new(Page::<M>::from_unused(i));
        }
        Self {
            range,
            _marker: core::marker::PhantomData,
        }
    }

    /// Get the start physical address of the contiguous pages.
    pub fn start_paddr(&self) -> Paddr {
        self.range.start
    }

    /// Get the length in bytes of the contiguous pages.
    pub fn len(&self) -> usize {
        self.range.end - self.range.start
    }
}

impl<M: PageMeta> From<Page<M>> for ContPages<M> {
    fn from(page: Page<M>) -> Self {
        let pa = page.paddr();
        let _ = ManuallyDrop::new(page);
        Self {
            range: pa..pa + PAGE_SIZE,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: PageMeta> From<ContPages<M>> for Vec<Page<M>> {
    fn from(pages: ContPages<M>) -> Self {
        let vector = pages
            .range
            .clone()
            .step_by(PAGE_SIZE)
            .map(|i|
            // SAFETY: for each page there would be a forgotten handle
            // when creating the `ContPages` object.
            unsafe { Page::<M>::from_raw(i) })
            .collect();
        let _ = ManuallyDrop::new(pages);
        vector
    }
}
