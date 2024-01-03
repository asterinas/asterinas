// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, Error};

use super::{frame::VmFrameFlags, frame_allocator, VmFrame, VmFrameVec, VmSegment};

/// Options for allocating physical memory pages (or frames).
///
/// All allocated frames are safe to use in the sense that they are
/// not _typed memory_. We define typed memory as the memory that
/// may store Rust objects or affect Rust memory safety, e.g.,
/// the code and data segments of the OS kernel, the stack and heap
/// allocated for the OS kernel.
pub struct VmAllocOptions {
    nframes: usize,
    is_contiguous: bool,
    uninit: bool,
}

impl VmAllocOptions {
    /// Creates new options for allocating the specified number of frames.
    pub fn new(nframes: usize) -> Self {
        Self {
            nframes,
            is_contiguous: false,
            uninit: false,
        }
    }

    /// Sets whether the allocated frames should be contiguous.
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

    /// Allocate a collection of page frames according to the given options.
    pub fn alloc(&self) -> Result<VmFrameVec> {
        let flags = self.flags();
        let frames = if self.is_contiguous {
            frame_allocator::alloc(self.nframes, flags).ok_or(Error::NoMemory)?
        } else {
            let mut frame_list = Vec::new();
            for _ in 0..self.nframes {
                frame_list.push(frame_allocator::alloc_single(flags).ok_or(Error::NoMemory)?);
            }
            VmFrameVec(frame_list)
        };
        if !self.uninit {
            frames.zero();
        }

        Ok(frames)
    }

    /// Allocate a single page frame according to the given options.
    pub fn alloc_single(&self) -> Result<VmFrame> {
        if self.nframes != 1 {
            return Err(Error::InvalidArgs);
        }

        let frame = frame_allocator::alloc_single(self.flags()).ok_or(Error::NoMemory)?;
        if !self.uninit {
            frame.zero();
        }

        Ok(frame)
    }

    /// Allocate a contiguous range of page frames according to the given options.
    ///
    /// The returned `VmSegment` contains at least one page frame.
    pub fn alloc_contiguous(&self) -> Result<VmSegment> {
        if !self.is_contiguous || self.nframes == 0 {
            return Err(Error::InvalidArgs);
        }

        let segment =
            frame_allocator::alloc_contiguous(self.nframes, self.flags()).ok_or(Error::NoMemory)?;
        if !self.uninit {
            segment.zero();
        }

        Ok(segment)
    }

    fn flags(&self) -> VmFrameFlags {
        VmFrameFlags::empty()
    }
}
