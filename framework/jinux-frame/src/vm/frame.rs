use alloc::vec;
use core::{
    iter::Iterator,
    ops::{BitAnd, BitOr, Not},
};

use crate::{arch::iommu, config::PAGE_SIZE, prelude::*, Error};

use super::frame_allocator;
use super::{Paddr, VmIo};

use pod::Pod;

/// A collection of page frames (physical memory pages).
///
/// For the most parts, `VmFrameVec` is like `Vec<VmFrame>`. But the
/// implementation may or may not be based on `Vec`. Having a dedicated
/// type to represent a series of page frames is convenient because,
/// more often than not, one needs to operate on a batch of frames rather
/// a single frame.
#[derive(Debug, Clone)]
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
        let page_size = options.page_size;
        let mut flags = VmFrameFlags::empty();
        if options.can_dma {
            flags.insert(VmFrameFlags::CAN_DMA);
        }
        let mut frames = if options.is_contiguous {
            frame_allocator::alloc_continuous(options.page_size, flags).ok_or(Error::NoMemory)?
        } else {
            let mut frame_list = Vec::new();
            for _ in 0..page_size {
                frame_list.push(frame_allocator::alloc(flags).ok_or(Error::NoMemory)?);
            }
            frame_list
        };
        if options.can_dma {
            for frame in frames.iter_mut() {
                // Safety: The address is controlled by frame allocator.
                unsafe {
                    if let Err(err) = iommu::map(frame.start_paddr(), frame.start_paddr()) {
                        match err {
                            // do nothing
                            iommu::IommuError::NoIommu => {}
                            iommu::IommuError::ModificationError(err) => {
                                panic!("iommu map error:{:?}", err)
                            }
                        }
                    }
                }
            }
        }
        let frame_vec = Self(frames);
        if !options.uninit {
            frame_vec.zero();
        }
        Ok(frame_vec)
    }

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

    /// Return IntoIterator for internal frames
    pub fn into_iter(self) -> alloc::vec::IntoIter<VmFrame> {
        self.0.into_iter()
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

impl VmIo for VmFrameVec {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut start = offset;
        let mut remain = buf.len();
        let mut processed = 0;
        for pa in self.0.iter() {
            if start >= PAGE_SIZE {
                start -= PAGE_SIZE;
            } else {
                let copy_len = (PAGE_SIZE - start).min(remain);
                let src = &mut buf[processed..processed + copy_len];
                let dst = unsafe { &pa.as_slice()[start..src.len() + start] };
                src.copy_from_slice(dst);
                processed += copy_len;
                remain -= copy_len;
                start = 0;
                if remain == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let mut start = offset;
        let mut remain = buf.len();
        let mut processed = 0;
        for pa in self.0.iter() {
            if start >= PAGE_SIZE {
                start -= PAGE_SIZE;
            } else {
                let copy_len = (PAGE_SIZE - start).min(remain);
                let src = &buf[processed..processed + copy_len];
                let dst = unsafe { &mut pa.as_slice()[start..src.len() + start] };
                dst.copy_from_slice(src);
                processed += copy_len;
                remain -= copy_len;
                start = 0;
                if remain == 0 {
                    break;
                }
            }
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

/// Options for allocating physical memory pages (or frames).
/// See `VmFrameVec::alloc`.
pub struct VmAllocOptions {
    page_size: usize,
    is_contiguous: bool,
    uninit: bool,
    can_dma: bool,
}

impl VmAllocOptions {
    /// Creates new options for allocating the specified number of frames.
    pub fn new(len: usize) -> Self {
        Self {
            page_size: len,
            is_contiguous: false,
            uninit: false,
            can_dma: false,
        }
    }

    /// Sets whether the allocated frames should be contiguous.
    ///
    /// If the physical address is set, then the frames must be contiguous.
    ///
    /// The default value is `false`.
    pub fn is_contiguous(&mut self, is_contiguous: bool) -> &mut Self {
        self.is_contiguous = is_contiguous;
        self
    }

    /// Sets whether the allocated frames should be uninitialized.
    ///
    /// If `uninit` is set as `false`, the frame will be zeroed once allocated.
    /// If `uninit` is set as `true`, the frame will **NOT** be zeroed and should *NOT* be read before writing.
    ///
    /// The default value is false.
    pub fn uninit(&mut self, uninit: bool) -> &mut Self {
        self.uninit = uninit;
        self
    }

    /// Sets whether the pages can be accessed by devices through
    /// Direct Memory Access (DMA).
    ///
    /// In a TEE environment, DMAable pages are untrusted pages shared with
    /// the VMM.
    pub fn can_dma(&mut self, can_dma: bool) -> &mut Self {
        self.can_dma = can_dma;
        self
    }
}

bitflags::bitflags! {
    pub(crate) struct VmFrameFlags : usize{
        const NEED_DEALLOC =    1 << 63;
        const CAN_DMA =         1 << 62;
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

    /// fill the frame with zero
    pub fn zero(&self) {
        unsafe {
            core::ptr::write_bytes(
                super::paddr_to_vaddr(self.start_paddr()) as *mut u8,
                0,
                PAGE_SIZE,
            )
        }
    }

    /// Returns whether the page frame is accessible by DMA.
    ///
    /// In a TEE environment, DMAable pages are untrusted pages shared with
    /// the VMM.
    pub fn can_dma(&self) -> bool {
        (*self.frame_index & VmFrameFlags::CAN_DMA.bits()) != 0
    }

    fn need_dealloc(&self) -> bool {
        (*self.frame_index & VmFrameFlags::NEED_DEALLOC.bits()) != 0
    }

    fn frame_index(&self) -> usize {
        (*self.frame_index).bitand(VmFrameFlags::all().bits().not())
    }

    // FIXME: need a sound reason for creating a mutable reference
    // for getting the content of the frame.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn as_slice(&self) -> &mut [u8] {
        core::slice::from_raw_parts_mut(
            super::paddr_to_vaddr(self.start_paddr()) as *mut u8,
            PAGE_SIZE,
        )
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

impl VmIo for VmFrame {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        if offset >= PAGE_SIZE || buf.len() + offset > PAGE_SIZE {
            Err(Error::InvalidArgs)
        } else {
            let dst = unsafe { &self.as_slice()[offset..buf.len() + offset] };
            buf.copy_from_slice(dst);
            Ok(())
        }
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        if offset >= PAGE_SIZE || buf.len() + offset > PAGE_SIZE {
            Err(Error::InvalidArgs)
        } else {
            let dst = unsafe { &mut self.as_slice()[offset..buf.len() + offset] };
            dst.copy_from_slice(buf);
            Ok(())
        }
    }

    /// Read a value of a specified type at a specified offset.
    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let paddr = self.start_paddr() + offset;
        Ok(unsafe { core::ptr::read(super::paddr_to_vaddr(paddr) as *const T) })
    }

    /// Write a value of a specified type at a specified offset.
    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        let paddr = self.start_paddr() + offset;
        unsafe { core::ptr::write(super::paddr_to_vaddr(paddr) as *mut T, *new_val) };
        Ok(())
    }
}

impl Drop for VmFrame {
    fn drop(&mut self) {
        if self.need_dealloc() && Arc::strong_count(&self.frame_index) == 1 {
            if self.can_dma() {
                if let Err(err) = iommu::unmap(self.start_paddr()) {
                    match err {
                        // do nothing
                        iommu::IommuError::NoIommu => {}
                        iommu::IommuError::ModificationError(err) => {
                            panic!("iommu map error:{:?}", err)
                        }
                    }
                }
            }
            unsafe {
                frame_allocator::dealloc(self.frame_index());
            }
        }
    }
}
