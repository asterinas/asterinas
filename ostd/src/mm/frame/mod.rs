// SPDX-License-Identifier: MPL-2.0

//! Untyped physical memory management.
//!
//! A frame is a special page that is _untyped_ memory.
//! It is used to store data irrelevant to the integrity of the kernel.
//! All pages mapped to the virtual address space of the users are backed by
//! frames. Frames, with all the properties of pages, can additionally be safely
//! read and written by the kernel or the user.

pub mod options;
pub mod segment;

use core::mem::ManuallyDrop;

pub use segment::Segment;

use super::page::{
    meta::{FrameMeta, MetaSlot, PageMeta, PageUsage},
    Page,
};
use crate::{
    mm::{
        io::{VmIo, VmReader, VmWriter},
        paddr_to_vaddr, HasPaddr, Paddr, PAGE_SIZE,
    },
    Error, Result,
};

/// An object-safe trait for the metadata of a frame.
///
/// The type of the metadata decides the type of the [`Frame`]. You can define
/// any type of frames by implementing this trait. The metadata is stored
/// globally and will be accessible by all of the [`Frame`] handles.
pub trait FrameMetaExt: Sync + core::fmt::Debug {
    /// The callback when the last reference to the frame is dropped.
    ///
    /// A reader is provided to allow the callback function to read the
    /// content of the frame that is about to be recycled.
    fn on_drop(&self, reader: VmReader);
}

/// A default frame metadata if you want to attach nothing to the frame.
#[derive(Debug, Default, Clone)]
#[repr(C)]
pub struct DefaultFrameMeta;

impl FrameMetaExt for DefaultFrameMeta {
    fn on_drop(&self, _reader: VmReader) {}
}

impl<M: FrameMetaExt + ?Sized> PageMeta for FrameMeta<M> {
    const USAGE: PageUsage = PageUsage::Frame;

    fn on_drop(page: &mut Page<Self>) {
        page.meta().as_ref().on_drop(page.reader());
    }
}

/// A handle to a physical memory page of untyped memory.
///
/// An instance of `Frame` is a handle to a page frame (a physical memory
/// page). A cloned `Frame` refers to the same page frame as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other. Behind the scene, a reference
/// counter is maintained for each page frame so that when all instances of
/// `Frame` that refer to the same page frame are dropped, the page frame
/// will be globally freed.
///
/// Any type of metadata can be attached to a frame if implementing the
/// [`FrameMetaExt`] trait. The type of metadata also defines the type of
/// the frame. For example, if you want a frame to become a cache of data on
/// disks and call such type of frame "disk cache frames", you can define the
/// types as follows:
///
/// ```compile_fail
/// use ostd::mm::frame::{Frame, FrameMetaExt};
///
/// #[derive(Debug, Default)]
/// struct DiskCacheMeta {
///     dirty: AtomicBool,
///     device: Arc<dyn BlkDevice>,
/// }
///
/// impl FrameMetaExt for DiskCacheMeta {
///     fn on_drop(frame: &mut Frame<Self>) {
///         // Write back the content to the disk if it is dirty.
///     }
/// }
///
/// type DiskCacheFrame = Frame<DiskCacheMeta>;
/// ```
pub type Frame<M = DefaultFrameMeta> = Page<FrameMeta<M>>;

impl<M: FrameMetaExt + ?Sized> Frame<M> {
    /// Returns the physical address of the page frame.
    pub fn start_paddr(&self) -> Paddr {
        self.paddr()
    }

    /// Returns the end physical address of the page frame.
    pub fn end_paddr(&self) -> Paddr {
        self.start_paddr() + PAGE_SIZE
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
    pub fn copy_from<MSrc: FrameMetaExt>(&self, src: &Frame<MSrc>) {
        if self.paddr() == src.paddr() {
            return;
        }
        // SAFETY: the source and the destination does not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.size());
        }
    }
}

impl<M: FrameMetaExt + ?Sized> HasPaddr for Frame<M> {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl<'a, M: FrameMetaExt + ?Sized> Frame<M> {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // SAFETY: the memory of the page is untyped, contiguous and is valid during `'a`.
        // Currently, only slice can generate `VmWriter` with typed memory, and this `Frame` cannot
        // generate or be generated from an alias slice, so the reader will not overlap with `VmWriter`
        // with typed memory.
        unsafe { VmReader::from_kernel_space(self.as_ptr(), self.size()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // SAFETY: the memory of the page is untyped, contiguous and is valid during `'a`.
        // Currently, only slice can generate `VmReader` with typed memory, and this `Frame` cannot
        // generate or be generated from an alias slice, so the writer will not overlap with `VmReader`
        // with typed memory.
        unsafe { VmWriter::from_kernel_space(self.as_mut_ptr(), self.size()) }
    }
}

impl<M: FrameMetaExt + ?Sized> VmIo for Frame<M> {
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

impl<M: FrameMetaExt + ?Sized> VmIo for alloc::vec::Vec<Frame<M>> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.len() * PAGE_SIZE {
            return Err(Error::InvalidArgs);
        }

        let num_skip_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_writer: VmWriter = buf.into();
        for frame in self.iter().skip(num_skip_pages) {
            let read_len = frame.reader().skip(start).read(&mut buf_writer);
            if read_len == 0 {
                break;
            }
            start = 0;
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.len() * PAGE_SIZE {
            return Err(Error::InvalidArgs);
        }

        let num_skip_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_reader: VmReader = buf.into();
        for frame in self.iter().skip(num_skip_pages) {
            let write_len = frame.writer().skip(start).write(&mut buf_reader);
            if write_len == 0 {
                break;
            }
            start = 0;
        }
        Ok(())
    }
}

// Here are implementations for `xarray`.

use core::{marker::PhantomData, ops::Deref};

/// `FrameRef` is a struct that can work as `&'a Frame`.
///
/// This is solely useful for [`crate::collections::xarray`].
pub struct FrameRef<'a, M: FrameMetaExt + ?Sized = DefaultFrameMeta> {
    inner: ManuallyDrop<Frame<M>>,
    _marker: PhantomData<&'a Frame<M>>,
}

impl<'a, M: FrameMetaExt + ?Sized> Deref for FrameRef<'a, M> {
    type Target = Frame<M>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `Frame` is essentially an `*const MetaSlot` that could be used as a `*const` pointer.
// The pointer is also aligned to 4.
unsafe impl<M: FrameMetaExt + ?Sized> xarray::ItemEntry for Frame<M> {
    type Ref<'a> = FrameRef<'a, M> where Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = self.ptr;
        let _ = ManuallyDrop::new(self);
        ptr as *const ()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            ptr: raw as *mut MetaSlot,
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Self {
                ptr: raw as *mut MetaSlot,
                _marker: PhantomData,
            }),
            _marker: PhantomData,
        }
    }
}
