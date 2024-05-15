// SPDX-License-Identifier: MPL-2.0

pub(crate) mod allocator;
pub(in crate::vm) mod meta;

use alloc::vec;
use core::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{self, Ordering},
};

use meta::{FrameMetaRef, FrameType};

use crate::{
    prelude::*,
    vm::{HasPaddr, PagingLevel, VmIo, VmReader, VmWriter, PAGE_SIZE},
    Error,
};

/// A collection of base page frames (regular physical memory pages).
///
/// For the most parts, `VmFrameVec` is like `Vec<VmFrame>`. But the
/// implementation may or may not be based on `Vec`. Having a dedicated
/// type to represent a series of page frames is convenient because,
/// more often than not, one needs to operate on a batch of frames rather
/// a single frame.
#[derive(Debug, Clone)]
pub struct VmFrameVec(pub(crate) Vec<VmFrame>);

impl VmFrameVec {
    pub fn get(&self, index: usize) -> Option<&VmFrame> {
        self.0.get(index)
    }

    /// returns an empty VmFrame vec
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn new_with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Pushs a new frame to the collection.
    pub fn push(&mut self, new_frame: VmFrame) {
        self.0.push(new_frame);
    }

    /// Pop a frame from the collection.
    pub fn pop(&mut self) -> Option<VmFrame> {
        self.0.pop()
    }

    /// Removes a frame at a position.
    pub fn remove(&mut self, at: usize) -> VmFrame {
        self.0.remove(at)
    }

    /// Append some frames.
    pub fn append(&mut self, more: &mut VmFrameVec) -> Result<()> {
        self.0.append(&mut more.0);
        Ok(())
    }

    /// Truncate some frames.
    ///
    /// If `new_len >= self.len()`, then this method has no effect.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.0.len() {
            return;
        }
        self.0.truncate(new_len)
    }

    /// Returns an iterator
    pub fn iter(&self) -> core::slice::Iter<'_, VmFrame> {
        self.0.iter()
    }

    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the frame collection is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of bytes.
    ///
    /// This method is equivalent to `self.len() * BASE_PAGE_SIZE`.
    pub fn nbytes(&self) -> usize {
        self.0.len() * PAGE_SIZE
    }

    pub fn from_one_frame(frame: VmFrame) -> Self {
        Self(vec![frame])
    }
}

impl IntoIterator for VmFrameVec {
    type Item = VmFrame;

    type IntoIter = alloc::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl VmIo for VmFrameVec {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }

        let num_unread_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_writer: VmWriter = buf.into();
        for frame in self.0.iter().skip(num_unread_pages) {
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
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }

        let num_unwrite_pages = offset / PAGE_SIZE;
        let mut start = offset % PAGE_SIZE;
        let mut buf_reader: VmReader = buf.into();
        for frame in self.0.iter().skip(num_unwrite_pages) {
            let write_len = frame.writer().skip(start).write(&mut buf_reader);
            if write_len == 0 {
                break;
            }
            start = 0;
        }
        Ok(())
    }
}

/// An iterator for frames.
pub struct FrameVecIter<'a> {
    frames: &'a VmFrameVec,
    current: usize,
}

impl<'a> FrameVecIter<'a> {
    pub fn new(frames: &'a VmFrameVec) -> Self {
        Self { frames, current: 0 }
    }
}

impl<'a> Iterator for FrameVecIter<'a> {
    type Item = &'a VmFrame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.frames.0.len() {
            return None;
        }
        Some(self.frames.0.get(self.current).unwrap())
    }
}

#[derive(Debug)]
/// A handle to a page frame.
///
/// The referenced page frame could either be huge or regular, which can be
/// told by the [`VmFrame::size`] method. It is ensured that there would be
/// only one TLB entry for such a frame if it is mapped to a virtual address
/// and the architecture supports huge TLB entries.
///
/// An instance of `VmFrame` is a handle to a page frame (a physical memory
/// page). A cloned `VmFrame` refers to the same page frame as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other. Behind the scene, a reference
/// counter is maintained for each page frame so that when all instances of
/// `VmFrame` that refer to the same page frame are dropped, the page frame
/// will be globally freed.
pub struct VmFrame {
    pub(crate) meta: FrameMetaRef,
}

unsafe impl Send for VmFrame {}
unsafe impl Sync for VmFrame {}

impl Clone for VmFrame {
    fn clone(&self) -> Self {
        self.meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        Self { meta: self.meta }
    }
}

impl HasPaddr for VmFrame {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl VmFrame {
    /// Creates a new `VmFrame` from the given physical address and level.
    ///
    /// # Panic
    ///
    /// The function panics if the given frame is not free or is managed
    /// by a non-free super page.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the given physical address is valid, and
    /// the page is free thus not accessed by any other objects or handles.
    pub(crate) unsafe fn from_free_raw(paddr: Paddr, level: PagingLevel) -> Self {
        let mut meta = FrameMetaRef::from_raw(paddr, level);
        assert!(matches!(meta.frame_type, FrameType::Free));
        meta.deref_mut().frame_type = FrameType::Anonymous;
        meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        Self { meta }
    }

    /// Returns the physical address of the page frame.
    pub fn start_paddr(&self) -> Paddr {
        self.meta.paddr()
    }

    pub fn size(&self) -> usize {
        self.meta.size()
    }

    pub fn end_paddr(&self) -> Paddr {
        self.start_paddr() + self.size()
    }

    pub fn as_ptr(&self) -> *const u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *mut u8
    }

    pub fn copy_from(&self, src: &VmFrame) {
        if self.meta == src.meta {
            return;
        }
        if self.size() != src.size() {
            panic!("The size of the source frame is different from the destination frame");
        }
        // Safety: the source and the destination does not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.size());
        }
    }
}

impl<'a> VmFrame {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // Safety: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmReader::from_raw_parts(self.as_ptr(), self.size()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // Safety: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmWriter::from_raw_parts_mut(self.as_mut_ptr(), self.size()) }
    }
}

impl VmIo for VmFrame {
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

impl Drop for VmFrame {
    fn drop(&mut self) {
        if self.meta.counter32_1.fetch_sub(1, Ordering::Release) == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            atomic::fence(Ordering::Acquire);
            // Safety: the reference counter is 1 before decremented, so this is the only
            // (exclusive) handle.
            unsafe { self.meta.deref_mut().frame_type = FrameType::Free };
            // Safety: the page frame is valid.
            unsafe {
                allocator::dealloc_contiguous(self.paddr() / PAGE_SIZE, self.size() / PAGE_SIZE);
            }
        }
    }
}

/// A handle to a contiguous range of page frames (physical memory pages).
///
/// The biggest difference between `VmSegment` and `VmFrameVec` is that
/// the page frames must be contiguous for `VmSegment`.
///
/// A cloned `VmSegment` refers to the same page frames as the original.
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
pub struct VmSegment {
    inner: VmSegmentInner,
    range: Range<usize>,
}

unsafe impl Send for VmSegment {}
unsafe impl Sync for VmSegment {}

#[derive(Debug)]
struct VmSegmentInner {
    meta: FrameMetaRef,
    nframes: usize,
}

impl Clone for VmSegmentInner {
    fn clone(&self) -> Self {
        self.meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        Self {
            meta: self.meta,
            nframes: self.nframes,
        }
    }
}

impl VmSegmentInner {
    /// Creates the inner part of 'VmSegment'.
    ///
    /// # Safety
    ///
    /// The constructor of 'VmSegment' ensures the safety.
    unsafe fn new(paddr: Paddr, nframes: usize) -> Self {
        assert_eq!(paddr % PAGE_SIZE, 0);
        let mut meta = FrameMetaRef::from_raw(paddr, 1);
        assert!(matches!(meta.frame_type, FrameType::Free));
        meta.deref_mut().frame_type = FrameType::Anonymous;
        meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        Self { meta, nframes }
    }

    fn start_frame_index(&self) -> usize {
        self.start_paddr() / PAGE_SIZE
    }

    fn start_paddr(&self) -> Paddr {
        self.meta.paddr()
    }
}

impl HasPaddr for VmSegment {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl VmSegment {
    /// Creates a new `VmSegment`.
    ///
    /// # Safety
    ///
    /// The given range of page frames must be contiguous and valid for use.
    /// The given range of page frames must not have been allocated before,
    /// as part of either a `VmFrame` or `VmSegment`.
    pub(crate) unsafe fn new(paddr: Paddr, nframes: usize) -> Self {
        Self {
            inner: VmSegmentInner::new(paddr, nframes),
            range: 0..nframes,
        }
    }

    /// Returns a part of the `VmSegment`.
    ///
    /// # Panic
    ///
    /// If `range` is not within the range of this `VmSegment`,
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
        self.inner.start_frame_index() + self.range.start
    }

    pub fn as_ptr(&self) -> *const u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *mut u8
    }
}

impl<'a> VmSegment {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // Safety: the memory of the page frames is contiguous and is valid during `'a`.
        unsafe { VmReader::from_raw_parts(self.as_ptr(), self.nbytes()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // Safety: the memory of the page frames is contiguous and is valid during `'a`.
        unsafe { VmWriter::from_raw_parts_mut(self.as_mut_ptr(), self.nbytes()) }
    }
}

impl VmIo for VmSegment {
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

impl Drop for VmSegment {
    fn drop(&mut self) {
        if self.inner.meta.counter32_1.fetch_sub(1, Ordering::Release) == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            atomic::fence(Ordering::Acquire);
            // Safety: the reference counter is 1 before decremented, so this is the only
            // (exclusive) handle.
            unsafe { self.inner.meta.deref_mut().frame_type = FrameType::Free };
            // Safety: the range of contiguous page frames is valid.
            unsafe {
                allocator::dealloc_contiguous(self.inner.start_frame_index(), self.inner.nframes);
            }
        }
    }
}

impl From<VmFrame> for VmSegment {
    fn from(frame: VmFrame) -> Self {
        let segment = Self {
            inner: VmSegmentInner {
                meta: frame.meta,
                nframes: 1,
            },
            range: 0..1,
        };
        let _ = ManuallyDrop::new(frame);
        segment
    }
}
