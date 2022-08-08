use core::iter::Iterator;

use crate::prelude::*;

/// A collection of page frames (physical memory pages).
///
/// For the most parts, `VmFrameVec` is like `Vec<VmFrame>`. But the
/// implementation may or may not be based on `Vec`. Having a dedicated
/// type to represent a series of page frames is convenient because,
/// more often than not, one needs to operate on a batch of frames rather
/// a single frame.
pub struct VmFrameVec(Vec<VmFrame>);

impl VmFrameVec {
    /// Allocate a collection of free frames according to the given options.
    ///
    /// All returned frames are safe to use in the sense that they are
    /// not _typed memory_. We define typed memory as the memory that
    /// may store Rust objects or affect Rust memory safety, e.g.,
    /// the code and data segments of the OS kernel, the stack and heap
    /// allocated for the OS kernel.
    ///
    /// For more information, see `VmAllocOptions`.
    pub fn allocate(options: &VmAllocOptions) -> Result<Self> {
        todo!()
    }

    /// Pushs a new frame to the collection.
    pub fn push(&mut self, new_frame: VmFrame) {
        todo!()
    }

    /// Pop a frame from the collection.
    pub fn pop(&mut self) -> Option<VmFrame> {
        todo!()
    }

    /// Removes a frame at a position.
    pub fn remove(&mut self, at: usize) -> VmFrame {
        todo!()
    }

    /// Append some frames.
    pub fn append(&mut self, more: VmFrameVec) -> Result<()> {
        todo!()
    }

    /// Truncate some frames.
    ///
    /// If `new_len >= self.len()`, then this method has no effect.
    pub fn truncate(&mut self, new_len: usize) {
        todo!()
    }

    /// Returns an iterator
    pub fn iter(&self) -> VmFrameVecIter<'_> {
        todo!()
    }

    /// Returns the number of frames.
    pub fn len(&self) -> usize {
        todo!()
    }

    /// Returns the number of bytes.
    ///
    /// This method is equivalent to `self.len() * PAGE_SIZE`.
    pub fn nbytes(&self) -> usize {
        todo!()
    }
}

/// An iterator for frames.
pub struct VmFrameVecIter<'a> {
    frames: &'a VmFrameVec,
    // more...
}

impl<'a> Iterator for VmFrameVecIter<'a> {
    type Item = &'a VmFrame;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

/// Options for allocating physical memory pages (or frames).
/// See `VmFrameVec::alloc`.
pub struct VmAllocOptions {}

impl VmAllocOptions {
    /// Creates new options for allocating the specified number of frames.
    pub fn new(len: usize) -> Self {
        todo!()
    }

    /// Sets the physical address of the first frame.
    ///
    /// If the physical address is given, then the allocated frames will be
    /// contiguous.
    ///
    /// The default value is `None`.
    pub fn paddr(&mut self, paddr: Option<Paddr>) -> &mut Self {
        todo!()
    }

    /// Sets whether the allocated frames should be contiguous.
    ///
    /// If the physical address is set, then the frames must be contiguous.
    ///
    /// The default value is `false`.
    pub fn is_contiguous(&mut self, is_contiguous: bool) -> &mut Self {
        todo!()
    }

    /// Sets whether the pages can be accessed by devices through
    /// Direct Memory Access (DMA).
    ///
    /// In a TEE environment, DMAable pages are untrusted pages shared with
    /// the VMM.
    pub fn can_dma(&mut self, can_dma: bool) -> &mut Self {
        todo!()
    }
}

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
pub struct VmFrame {}

impl VmFrame {
    /// Creates a new VmFrame.
    ///
    /// # Safety
    ///
    /// The given physical address must be valid for use.
    pub(crate) unsafe fn new(paddr: Paddr) -> Self {
        todo!()
    }

    /// Returns the physical address of the page frame.
    pub fn paddr(&self) -> Paddr {
        todo!()
    }

    /// Returns whether the page frame is accessible by DMA.
    ///
    /// In a TEE environment, DMAable pages are untrusted pages shared with
    /// the VMM.
    pub fn can_dma(&self) -> bool {
        todo!()
    }
}

impl Clone for VmFrame {
    fn clone(&self) -> Self {
        todo!("inc ref cnt")
    }
}

impl Drop for VmFrame {
    fn drop(&mut self) {
        todo!("dec ref cnt and if zero, free the page frame")
    }
}
