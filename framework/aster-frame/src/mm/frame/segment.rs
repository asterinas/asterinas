// SPDX-License-Identifier: MPL-2.0

//! A contiguous range of page frames.

use alloc::sync::Arc;
use core::ops::Range;

use super::Frame;
use crate::{
    mm::{
        page::{cont_pages::ContPages, meta::FrameMeta, Page},
        HasPaddr, Paddr, VmIo, VmReader, VmWriter, PAGE_SIZE,
    },
    Error, Result,
};

/// A handle to a contiguous range of page frames (physical memory pages).
///
/// The biggest difference between `Segment` and [`FrameVec`] is that
/// the page frames must be contiguous for `Segment`.
///
/// A cloned `Segment` refers to the same page frames as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other.
///
/// [`FrameVec`]: crate::mm::FrameVec
///
/// #Example
///
/// ```rust
/// let vm_segment = FrameAllocOptions::new(2)
///     .is_contiguous(true)
///     .alloc_contiguous()?;
/// vm_segment.write_bytes(0, buf)?;
/// ```
#[derive(Debug, Clone)]
pub struct Segment {
    inner: Arc<ContPages<FrameMeta>>,
    range: Range<usize>,
}

impl HasPaddr for Segment {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl Segment {
    /// Returns a part of the `Segment`.
    ///
    /// # Panics
    ///
    /// If `range` is not within the range of this `Segment`,
    /// then the method panics.
    pub fn range(&self, range: Range<usize>) -> Self {
        let orig_range = &self.range;
        let adj_range = (range.start + orig_range.start)..(range.end + orig_range.start);
        assert!(!adj_range.is_empty() && adj_range.end <= orig_range.end);

        Self {
            inner: self.inner.clone(),
            range: adj_range,
        }
    }

    /// Returns the start physical address.
    pub fn start_paddr(&self) -> Paddr {
        self.start_frame_index() * PAGE_SIZE
    }

    /// Returns the end physical address.
    pub fn end_paddr(&self) -> Paddr {
        (self.start_frame_index() + self.nframes()) * PAGE_SIZE
    }

    /// Returns the number of page frames.
    pub fn nframes(&self) -> usize {
        self.range.len()
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.nframes() * PAGE_SIZE
    }

    fn start_frame_index(&self) -> usize {
        self.inner.start_paddr() / PAGE_SIZE + self.range.start
    }

    /// Returns a raw pointer to the starting virtual address of the `Segment`.
    pub fn as_ptr(&self) -> *const u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    /// Returns a mutable raw pointer to the starting virtual address of the `Segment`.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *mut u8
    }
}

impl<'a> Segment {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // SAFETY: the memory of the page frames is contiguous and is valid during `'a`.
        unsafe { VmReader::from_raw_parts(self.as_ptr(), self.nbytes()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // SAFETY: the memory of the page frames is contiguous and is valid during `'a`.
        unsafe { VmWriter::from_raw_parts_mut(self.as_mut_ptr(), self.nbytes()) }
    }
}

impl VmIo for Segment {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self.reader().skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self.writer().skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

impl From<Frame> for Segment {
    fn from(frame: Frame) -> Self {
        Self {
            inner: Arc::new(Page::<FrameMeta>::from(frame).into()),
            range: 0..1,
        }
    }
}

impl From<ContPages<FrameMeta>> for Segment {
    fn from(cont_pages: ContPages<FrameMeta>) -> Self {
        let len = cont_pages.len();
        Self {
            inner: Arc::new(cont_pages),
            range: 0..len / PAGE_SIZE,
        }
    }
}
