// SPDX-License-Identifier: MPL-2.0

use core::mem::ManuallyDrop;

use super::{
    allocator,
    meta::{FrameMeta, PageMeta, PageUsage},
    Page,
};
use crate::{
    mm::{
        io::{VmIo, VmReader, VmWriter},
        paddr_to_vaddr, HasPaddr, Paddr, PagingLevel, PAGE_SIZE,
    },
    Error, Result,
};

/// A handle to a page frame.
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
    pub(in crate::mm) page: Page<FrameMeta>,
}

impl HasPaddr for Frame {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl Frame {
    /// Returns the physical address of the page frame.
    pub fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }

    pub fn end_paddr(&self) -> Paddr {
        self.start_paddr() + PAGE_SIZE
    }

    /// Get the paging level of the frame.
    ///
    /// This is the level of the page table entry that maps the frame,
    /// which determines the size of the frame.
    ///
    /// Currently, the level is always 1, which means the frame is a regular
    /// page frame.
    pub(crate) fn level(&self) -> PagingLevel {
        1
    }

    pub fn size(&self) -> usize {
        PAGE_SIZE
    }

    pub fn as_ptr(&self) -> *const u8 {
        paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        paddr_to_vaddr(self.start_paddr()) as *mut u8
    }

    pub fn copy_from(&self, src: &Frame) {
        if self.paddr() == src.paddr() {
            return;
        }
        // SAFETY: the source and the destination does not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.size());
        }
    }
}

impl<'a> Frame {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // SAFETY: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmReader::from_raw_parts(self.as_ptr(), self.size()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // SAFETY: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmWriter::from_raw_parts_mut(self.as_mut_ptr(), self.size()) }
    }
}

impl VmIo for Frame {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.size() {
            return Err(Error::InvalidArgs);
        }
        let len = self.reader().skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.size() {
            return Err(Error::InvalidArgs);
        }
        let len = self.writer().skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

impl PageMeta for FrameMeta {
    const USAGE: PageUsage = PageUsage::Frame;

    fn on_drop(page: &mut Page<Self>) {
        unsafe { allocator::dealloc(page.paddr() / PAGE_SIZE, 1) };
    }
}

// Here are implementations for `xarray`.

use core::{marker::PhantomData, ops::Deref};

/// `VmFrameRef` is a struct that can work as `&'a Frame`.
pub struct VmFrameRef<'a> {
    inner: ManuallyDrop<Frame>,
    _marker: PhantomData<&'a Frame>,
}

impl<'a> Deref for VmFrameRef<'a> {
    type Target = Frame;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `Frame` is essentially an `*const FrameMeta` that could be used as a `*const` pointer.
// The pointer is also aligned to 4.
unsafe impl xarray::ItemEntry for Frame {
    type Ref<'a> = VmFrameRef<'a> where Self: 'a;

    fn into_raw(self) -> *const () {
        self.page.forget() as *const ()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            page: Page::<FrameMeta>::restore(raw as Paddr),
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Frame::from_raw(raw)),
            _marker: PhantomData,
        }
    }
}
