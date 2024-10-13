// SPDX-License-Identifier: MPL-2.0

//! Untyped physical memory management.
//!
//! A frame is a special page that is _untyped_ memory.
//! It is used to store data irrelevant to the integrity of the kernel.
//! All pages mapped to the virtual address space of the users are backed by
//! frames. Frames, with all the properties of pages, can additionally be safely
//! read and written by the kernel or the user.

pub mod options;
mod segment;

use core::mem::ManuallyDrop;

pub use segment::Segment;

use super::page::{
    meta::{FrameMeta, MetaSlot, PageMeta, PageUsage},
    DynPage, Page,
};
use crate::{
    mm::{
        io::{FallibleVmRead, FallibleVmWrite, VmIo, VmReader, VmWriter},
        paddr_to_vaddr, HasPaddr, Infallible, Paddr, PAGE_SIZE,
    },
    Error, Result,
};

/// A handle to a physical memory page of untyped memory.
///
/// An instance of `Frame` is a handle to a page frame (a physical memory
/// page). A cloned `Frame` refers to the same page frame as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other. Behind the scene, a reference
/// counter is maintained for each page frame so that when all instances of
/// `Frame` that refer to the same page frame are dropped, the page frame
/// will be globally freed.
#[derive(Debug, Clone)]
pub struct Frame {
    page: Page<FrameMeta>,
}

impl Frame {
    /// Returns the physical address of the page frame.
    pub fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }

    /// Returns the end physical address of the page frame.
    pub fn end_paddr(&self) -> Paddr {
        self.start_paddr() + PAGE_SIZE
    }

    /// Returns the size of the frame
    pub const fn size(&self) -> usize {
        self.page.size()
    }

    /// Returns a raw pointer to the starting virtual address of the frame.
    pub fn as_ptr(&self) -> *const u8 {
        paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    /// Returns a mutable raw pointer to the starting virtual address of the frame.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        paddr_to_vaddr(self.start_paddr()) as *mut u8
    }

    /// Copies the content of `src` to the frame.
    pub fn copy_from(&self, src: &Frame) {
        if self.paddr() == src.paddr() {
            return;
        }
        // SAFETY: the source and the destination does not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.size());
        }
    }

    /// Get the reference count of the frame.
    ///
    /// It returns the number of all references to the page, including all the
    /// existing page handles ([`Frame`]) and all the mappings in the page
    /// table that points to the page.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.page.reference_count()
    }
}

impl From<Page<FrameMeta>> for Frame {
    fn from(page: Page<FrameMeta>) -> Self {
        Self { page }
    }
}

impl TryFrom<DynPage> for Frame {
    type Error = DynPage;

    /// Try converting a [`DynPage`] into the statically-typed [`Frame`].
    ///
    /// If the dynamic page is not used as an untyped page frame, it will
    /// return the dynamic page itself as is.
    fn try_from(page: DynPage) -> core::result::Result<Self, Self::Error> {
        page.try_into().map(|p: Page<FrameMeta>| p.into())
    }
}

impl From<Frame> for Page<FrameMeta> {
    fn from(frame: Frame) -> Self {
        frame.page
    }
}

impl HasPaddr for Frame {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl<'a> Frame {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a, Infallible> {
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The frame is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the frame.
        unsafe { VmReader::from_kernel_space(self.as_ptr(), self.size()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a, Infallible> {
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The frame is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the frame.
        unsafe { VmWriter::from_kernel_space(self.as_mut_ptr(), self.size()) }
    }
}

impl VmIo for Frame {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(read_len).ok_or(Error::Overflow)?;
        if max_offset > self.size() {
            return Err(Error::InvalidArgs);
        }
        let len = self
            .reader()
            .skip(offset)
            .read_fallible(writer)
            .map_err(|(e, _)| e)?;
        debug_assert!(len == read_len);
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain().min(self.size().saturating_sub(offset));
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(write_len).ok_or(Error::Overflow)?;
        if max_offset > self.size() {
            return Err(Error::InvalidArgs);
        }
        let len = self
            .writer()
            .skip(offset)
            .write_fallible(reader)
            .map_err(|(e, _)| e)?;
        debug_assert!(len == write_len);
        Ok(())
    }
}

impl PageMeta for FrameMeta {
    const USAGE: PageUsage = PageUsage::Frame;

    fn on_drop(_page: &mut Page<Self>) {
        // Nothing should be done so far since dropping the page would
        // have all taken care of.
    }
}

// Here are implementations for `xarray`.

use core::{marker::PhantomData, ops::Deref};

/// `FrameRef` is a struct that can work as `&'a Frame`.
///
/// This is solely useful for [`crate::collections::xarray`].
pub struct FrameRef<'a> {
    inner: ManuallyDrop<Frame>,
    _marker: PhantomData<&'a Frame>,
}

impl Deref for FrameRef<'_> {
    type Target = Frame;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `Frame` is essentially an `*const MetaSlot` that could be used as a `*const` pointer.
// The pointer is also aligned to 4.
unsafe impl xarray::ItemEntry for Frame {
    type Ref<'a>
        = FrameRef<'a>
    where
        Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = self.page.ptr;
        core::mem::forget(self);
        ptr as *const ()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            page: Page::<FrameMeta> {
                ptr: raw as *mut MetaSlot,
                _marker: PhantomData,
            },
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Frame::from_raw(raw)),
            _marker: PhantomData,
        }
    }
}
