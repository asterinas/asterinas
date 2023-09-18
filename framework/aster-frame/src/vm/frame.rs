use alloc::vec;
use core::{
    iter::Iterator,
    marker::PhantomData,
    ops::{BitAnd, BitOr, Not, Range},
};

use crate::{config::PAGE_SIZE, prelude::*, Error};

use super::{frame_allocator, HasPaddr};
use super::{Paddr, VmIo};

/// A collection of page frames (physical memory pages).
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

    /// returns an empty vmframe vec
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

    /// zero all internal vm frames
    pub fn zero(&self) {
        self.0.iter().for_each(|vm_frame| vm_frame.zero())
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
    /// This method is equivalent to `self.len() * PAGE_SIZE`.
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
        if buf.len() + offset > self.nbytes() {
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
        if buf.len() + offset > self.nbytes() {
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
pub struct VmFrameVecIter<'a> {
    frames: &'a VmFrameVec,
    current: usize,
    // more...
}

impl<'a> VmFrameVecIter<'a> {
    pub fn new(frames: &'a VmFrameVec) -> Self {
        Self { frames, current: 0 }
    }
}

impl<'a> Iterator for VmFrameVecIter<'a> {
    type Item = &'a VmFrame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.frames.0.len() {
            return None;
        }
        Some(self.frames.0.get(self.current).unwrap())
    }
}

bitflags::bitflags! {
    pub(crate) struct VmFrameFlags : usize {
        const NEED_DEALLOC =    1 << 63;
    }
}

#[derive(Debug)]
/// A handle to a page frame.
///
/// An instance of `VmFrame` is a handle to a page frame (a physical memory
/// page). A cloned `VmFrame` refers to the same page frame as the original.
/// As the original and cloned instances point to the same physical address,  
/// they are treated as equal to each other. Behind the scene,
/// a reference counter is maintained for each page frame so that
/// when all instances of `VmFrame` that refer to the
/// same page frame are dropped, the page frame will be freed.
/// Free page frames are allocated in bulk by `VmFrameVec::allocate`.
pub struct VmFrame {
    pub(crate) frame_index: Arc<Paddr>,
}

impl Clone for VmFrame {
    fn clone(&self) -> Self {
        Self {
            frame_index: self.frame_index.clone(),
        }
    }
}

impl HasPaddr for VmFrame {
    fn paddr(&self) -> Paddr {
        self.start_paddr()
    }
}

impl VmFrame {
    /// Creates a new VmFrame.
    ///
    /// # Safety
    ///
    /// The given physical address must be valid for use.
    pub(crate) unsafe fn new(paddr: Paddr, flags: VmFrameFlags) -> Self {
        assert_eq!(paddr % PAGE_SIZE, 0);
        Self {
            frame_index: Arc::new((paddr / PAGE_SIZE).bitor(flags.bits)),
        }
    }

    /// Returns the physical address of the page frame.
    pub fn start_paddr(&self) -> Paddr {
        self.frame_index() * PAGE_SIZE
    }

    pub fn end_paddr(&self) -> Paddr {
        (self.frame_index() + 1) * PAGE_SIZE
    }

    /// Fills the frame with zero.
    pub fn zero(&self) {
        // Safety: The range of memory is valid for writes of one page data.
        unsafe { core::ptr::write_bytes(self.as_mut_ptr(), 0, PAGE_SIZE) }
    }

    fn need_dealloc(&self) -> bool {
        (*self.frame_index & VmFrameFlags::NEED_DEALLOC.bits()) != 0
    }

    fn frame_index(&self) -> usize {
        (*self.frame_index).bitand(VmFrameFlags::all().bits().not())
    }

    pub fn as_ptr(&self) -> *const u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *const u8
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        super::paddr_to_vaddr(self.start_paddr()) as *mut u8
    }

    pub fn copy_from_frame(&self, src: &VmFrame) {
        if Arc::ptr_eq(&self.frame_index, &src.frame_index) {
            return;
        }

        // Safety: src and dst is not overlapped.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), PAGE_SIZE);
        }
    }
}

impl<'a> VmFrame {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        // Safety: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmReader::from_raw_parts(self.as_ptr(), PAGE_SIZE) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        // Safety: the memory of the page is contiguous and is valid during `'a`.
        unsafe { VmWriter::from_raw_parts_mut(self.as_mut_ptr(), PAGE_SIZE) }
    }
}

impl VmIo for VmFrame {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        if buf.len() + offset > PAGE_SIZE {
            return Err(Error::InvalidArgs);
        }
        let len = self.reader().skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        if buf.len() + offset > PAGE_SIZE {
            return Err(Error::InvalidArgs);
        }
        let len = self.writer().skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

impl Drop for VmFrame {
    fn drop(&mut self) {
        if self.need_dealloc() && Arc::strong_count(&self.frame_index) == 1 {
            // Safety: the frame index is valid.
            unsafe {
                frame_allocator::dealloc_single(self.frame_index());
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
    inner: Arc<Inner>,
    range: Range<usize>,
}

#[derive(Debug)]
struct Inner {
    start_frame_index: Paddr,
    nframes: usize,
}

impl Inner {
    /// Creates the inner part of 'VmSegment'.
    ///
    /// # Safety
    ///
    /// The constructor of 'VmSegment' ensures the safety.
    unsafe fn new(paddr: Paddr, nframes: usize, flags: VmFrameFlags) -> Self {
        assert_eq!(paddr % PAGE_SIZE, 0);
        Self {
            start_frame_index: (paddr / PAGE_SIZE).bitor(flags.bits),
            nframes,
        }
    }

    fn start_frame_index(&self) -> usize {
        self.start_frame_index
            .bitand(VmFrameFlags::all().bits().not())
    }

    fn start_paddr(&self) -> Paddr {
        self.start_frame_index() * PAGE_SIZE
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
    pub(crate) unsafe fn new(paddr: Paddr, nframes: usize, flags: VmFrameFlags) -> Self {
        Self {
            inner: Arc::new(Inner::new(paddr, nframes, flags)),
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

    /// Fills the page frames with zero.
    pub fn zero(&self) {
        // Safety: The range of memory is valid for writes of `self.nbytes()` data.
        unsafe { core::ptr::write_bytes(self.as_mut_ptr(), 0, self.nbytes()) }
    }

    fn need_dealloc(&self) -> bool {
        (self.inner.start_frame_index & VmFrameFlags::NEED_DEALLOC.bits()) != 0
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
        if buf.len() + offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self.reader().skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        if buf.len() + offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self.writer().skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

impl Drop for VmSegment {
    fn drop(&mut self) {
        if self.need_dealloc() && Arc::strong_count(&self.inner) == 1 {
            // Safety: the range of contiguous page frames is valid.
            unsafe {
                frame_allocator::dealloc_contiguous(
                    self.inner.start_frame_index(),
                    self.inner.nframes,
                );
            }
        }
    }
}

/// VmReader is a reader for reading data from a contiguous range of memory.
///
/// # Example
///
/// ```rust
/// impl VmIo for VmFrame {
///     fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
///         if buf.len() + offset > PAGE_SIZE {
///             return Err(Error::InvalidArgs);
///         }
///         let len = self.reader().skip(offset).read(&mut buf.into());
///         debug_assert!(len == buf.len());
///         Ok(())
///     }
/// }
/// ```
pub struct VmReader<'a> {
    cursor: *const u8,
    end: *const u8,
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> VmReader<'a> {
    /// Constructs a VmReader from a pointer and a length.
    ///
    /// # Safety
    ///
    /// User must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// User must ensure the memory is valid during the entire period of `'a`.
    pub const unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Self {
        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Returns the number of bytes for the remaining data.
    pub const fn remain(&self) -> usize {
        // Safety: the end is equal to or greater than the cursor.
        unsafe { self.end.sub_ptr(self.cursor) }
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to read.
    pub const fn cursor(&self) -> *const u8 {
        self.cursor
    }

    /// Returns if it has remaining data to read.
    pub const fn has_remain(&self) -> bool {
        self.remain() > 0
    }

    /// Limits the length of remaining data.
    ///
    /// This method ensures the postcondition of `self.remain() <= max_remain`.
    pub const fn limit(mut self, max_remain: usize) -> Self {
        if max_remain < self.remain() {
            // Safety: the new end is less than the old end.
            unsafe { self.end = self.cursor.add(max_remain) };
        }
        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of remaining data is decreased accordingly.
    ///
    /// # Panic
    ///
    /// If `nbytes` is greater than `self.remain()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.remain());

        // Safety: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }

    /// Reads all data into the writer until one of the two conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    ///
    /// Returns the number of bytes read.
    ///
    /// It pulls the number of bytes data from the reader and
    /// fills in the writer with the number of bytes.
    pub fn read(&mut self, writer: &mut VmWriter<'_>) -> usize {
        let copy_len = self.remain().min(writer.avail());
        if copy_len == 0 {
            return 0;
        }

        // Safety: the memory range is valid since `copy_len` is the minimum
        // of the reader's remaining data and the writer's available space.
        unsafe {
            core::ptr::copy(self.cursor, writer.cursor, copy_len);
            self.cursor = self.cursor.add(copy_len);
            writer.cursor = writer.cursor.add(copy_len);
        }
        copy_len
    }
}

impl<'a> From<&'a [u8]> for VmReader<'a> {
    fn from(slice: &'a [u8]) -> Self {
        // Safety: the range of memory is contiguous and is valid during `'a`.
        unsafe { Self::from_raw_parts(slice.as_ptr(), slice.len()) }
    }
}

/// VmWriter is a writer for writing data to a contiguous range of memory.
///
/// # Example
///
/// ```rust
/// impl VmIo for VmFrame {
///     fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
///         if buf.len() + offset > PAGE_SIZE {
///             return Err(Error::InvalidArgs);
///         }
///         let len = self.writer().skip(offset).write(&mut buf.into());
///         debug_assert!(len == buf.len());
///         Ok(())
///     }
/// }
/// ```
pub struct VmWriter<'a> {
    cursor: *mut u8,
    end: *mut u8,
    phantom: PhantomData<&'a mut [u8]>,
}

impl<'a> VmWriter<'a> {
    /// Constructs a VmWriter from a pointer and a length.
    ///
    /// # Safety
    ///
    /// User must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// User must ensure the memory is valid during the entire period of `'a`.
    pub const unsafe fn from_raw_parts_mut(ptr: *mut u8, len: usize) -> Self {
        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Returns the number of bytes for the available space.
    pub const fn avail(&self) -> usize {
        // Safety: the end is equal to or greater than the cursor.
        unsafe { self.end.sub_ptr(self.cursor) }
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to write.
    pub const fn cursor(&self) -> *mut u8 {
        self.cursor
    }

    /// Returns if it has avaliable space to write.
    pub const fn has_avail(&self) -> bool {
        self.avail() > 0
    }

    /// Limits the length of available space.
    ///
    /// This method ensures the postcondition of `self.avail() <= max_avail`.
    pub const fn limit(mut self, max_avail: usize) -> Self {
        if max_avail < self.avail() {
            // Safety: the new end is less than the old end.
            unsafe { self.end = self.cursor.add(max_avail) };
        }
        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of available space is decreased accordingly.
    ///
    /// # Panic
    ///
    /// If `nbytes` is greater than `self.avail()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.avail());

        // Safety: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }

    /// Writes data from the reader until one of the two conditions is met:
    /// 1. The writer has no available space.
    /// 2. The reader has no remaining data.
    ///
    /// Returns the number of bytes written.
    ///
    /// It pulls the number of bytes data from the reader and
    /// fills in the writer with the number of bytes.
    pub fn write(&mut self, reader: &mut VmReader<'_>) -> usize {
        let copy_len = self.avail().min(reader.remain());
        if copy_len == 0 {
            return 0;
        }

        // Safety: the memory range is valid since `copy_len` is the minimum
        // of the reader's remaining data and the writer's available space.
        unsafe {
            core::ptr::copy(reader.cursor, self.cursor, copy_len);
            self.cursor = self.cursor.add(copy_len);
            reader.cursor = reader.cursor.add(copy_len);
        }
        copy_len
    }
}

impl<'a> From<&'a mut [u8]> for VmWriter<'a> {
    fn from(slice: &'a mut [u8]) -> Self {
        // Safety: the range of memory is contiguous and is valid during `'a`.
        unsafe { Self::from_raw_parts_mut(slice.as_mut_ptr(), slice.len()) }
    }
}
