// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use super::{
    allocator,
    meta::{PageMeta, PageUsage, SegmentHeadMeta},
    Frame, Page,
};
use crate::{
    mm::{HasPaddr, Paddr, VmIo, VmReader, VmWriter, PAGE_SIZE},
    Error, Result,
};

/// A handle to a contiguous range of page frames (physical memory pages).
///
/// The biggest difference between `Segment` and `VmFrameVec` is that
/// the page frames must be contiguous for `Segment`.
///
/// A cloned `Segment` refers to the same page frames as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other.
///
/// #Example
///
/// ```rust
/// let vm_segment = VmAllocOptions::new(2)
///     .is_contiguous(true)
///     .alloc_contiguous()?;
/// vm_segment.write_bytes(0, buf)?;
/// ```
#[derive(Debug, Clone)]
pub struct Segment {
    head_page: Page<SegmentHeadMeta>,
    range: Range<usize>,
}

impl HasPaddr for Segment {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl Segment {
    /// Creates a new `Segment`.
    ///
    /// # Safety
    ///
    /// The given range of page frames must be contiguous and valid for use.
    /// The given range of page frames must not have been allocated before,
    /// as part of either a `Frame` or `Segment`.
    pub(crate) unsafe fn new(paddr: Paddr, nframes: usize) -> Self {
        let mut head = Page::<SegmentHeadMeta>::from_unused(paddr).unwrap();
        head.meta_mut().seg_len = (nframes * PAGE_SIZE) as u64;
        Self {
            head_page: head,
            range: 0..nframes,
        }
    }

    /// Returns a part of the `Segment`.
    ///
    /// # Panic
    ///
    /// If `range` is not within the range of this `Segment`,
    /// then the method panics.
    pub fn range(&self, range: Range<usize>) -> Self {
        let orig_range = &self.range;
        let adj_range = (range.start + orig_range.start)..(range.end + orig_range.start);
        assert!(!adj_range.is_empty() && adj_range.end <= orig_range.end);

        Self {
            head_page: self.head_page.clone(),
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
        self.head_page.paddr() / PAGE_SIZE + self.range.start
    }

    pub fn as_ptr(&self) -> *const u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *const u8
    }

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

impl PageMeta for SegmentHeadMeta {
    const USAGE: PageUsage = PageUsage::SegmentHead;

    fn on_drop(page: &mut Page<Self>) {
        let nframes = page.meta().seg_len as usize / PAGE_SIZE;
        let start_index = page.paddr() / PAGE_SIZE;
        unsafe { allocator::dealloc(start_index, nframes) };
    }
}

impl From<Frame> for Segment {
    fn from(frame: Frame) -> Self {
        Self {
            head_page: frame.page.into(),
            range: 0..1,
        }
    }
}
