// SPDX-License-Identifier: MPL-2.0

//! A contiguous segment of untyped memory pages.

use core::ops::Range;

use crate::{
    mm::{
        io::{FallibleVmRead, FallibleVmWrite},
        page::{meta::FrameMeta, ContPages},
        Frame, HasPaddr, Infallible, Paddr, VmIo, VmReader, VmWriter,
    },
    Error, Result,
};

/// A contiguous segment of untyped memory pages.
///
/// A [`Segment`] object is a handle to a contiguous range of untyped memory
/// pages, and the underlying pages can be shared among multiple threads.
/// [`Segment::slice`] can be used to clone a slice of the segment (also can be
/// used to clone the entire range). Reference counts are maintained for each
/// page in the segment. So cloning the handle may not be cheap as it
/// increments the reference count of all the cloned pages.
///
/// Other [`Frame`] handles can also refer to the pages in the segment. And
/// the segment can be iterated over to get all the frames in it.
///
/// To allocate a segment, use [`crate::mm::FrameAllocator`].
///
/// # Example
///
/// ```rust
/// let vm_segment = FrameAllocOptions::new(2)
///     .is_contiguous(true)
///     .alloc_contiguous()?;
/// vm_segment.write_bytes(0, buf)?;
/// ```
#[derive(Debug)]
pub struct Segment {
    pages: ContPages<FrameMeta>,
}

impl HasPaddr for Segment {
    fn paddr(&self) -> Paddr {
        self.pages.start_paddr()
    }
}

impl Clone for Segment {
    fn clone(&self) -> Self {
        Self {
            pages: self.pages.clone(),
        }
    }
}

impl Segment {
    /// Returns the start physical address.
    pub fn start_paddr(&self) -> Paddr {
        self.pages.start_paddr()
    }

    /// Returns the end physical address.
    pub fn end_paddr(&self) -> Paddr {
        self.pages.end_paddr()
    }

    /// Returns the number of bytes in it.
    pub fn nbytes(&self) -> usize {
        self.pages.nbytes()
    }

    /// Split the segment into two at the given byte offset from the start.
    ///
    /// The resulting segments cannot be empty. So the byte offset cannot be
    /// neither zero nor the length of the segment.
    ///
    /// # Panics
    ///
    /// The function panics if the byte offset is out of bounds, at either ends, or
    /// not base-page-aligned.
    pub fn split(self, offset: usize) -> (Self, Self) {
        let (left, right) = self.pages.split(offset);
        (Self { pages: left }, Self { pages: right })
    }

    /// Get an extra handle to the segment in the byte range.
    ///
    /// The sliced byte range in indexed by the offset from the start of the
    /// segment. The resulting segment holds extra reference counts.
    ///
    /// # Panics
    ///
    /// The function panics if the byte range is out of bounds, or if any of
    /// the ends of the byte range is not base-page aligned.
    pub fn slice(&self, range: &Range<usize>) -> Self {
        Self {
            pages: self.pages.slice(range),
        }
    }

    /// Gets a [`VmReader`] to read from the segment from the beginning to the end.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        let ptr = super::paddr_to_vaddr(self.start_paddr()) as *const u8;
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The segment is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the segment.
        unsafe { VmReader::from_kernel_space(ptr, self.nbytes()) }
    }

    /// Gets a [`VmWriter`] to write to the segment from the beginning to the end.
    pub fn writer(&self) -> VmWriter<'_, Infallible> {
        let ptr = super::paddr_to_vaddr(self.start_paddr()) as *mut u8;
        // SAFETY:
        // - The memory range points to untyped memory.
        // - The segment is alive during the lifetime `'a`.
        // - Using `VmReader` and `VmWriter` is the only way to access the segment.
        unsafe { VmWriter::from_kernel_space(ptr, self.nbytes()) }
    }
}

impl From<Frame> for Segment {
    fn from(frame: Frame) -> Self {
        Self {
            pages: ContPages::from(frame.page),
        }
    }
}

impl From<ContPages<FrameMeta>> for Segment {
    fn from(pages: ContPages<FrameMeta>) -> Self {
        Self { pages }
    }
}

impl VmIo for Segment {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail();
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(read_len).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
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
        let write_len = reader.remain();
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(reader.remain()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
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

impl Iterator for Segment {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        self.pages.next().map(|page| Frame { page })
    }
}
