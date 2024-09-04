// SPDX-License-Identifier: MPL-2.0

//! Options for allocating frames

use super::{Frame, Segment};
use crate::{
    mm::{
        page::{
            self,
            meta::{FrameMeta, FrameMetaBox},
        },
        PAGE_SIZE,
    },
    prelude::*,
    Error,
};

/// Options for allocating physical memory pages (or frames).
///
/// All allocated frames are safe to use in the sense that they are
/// not _typed memory_. We define typed memory as the memory that
/// may store Rust objects or affect Rust memory safety, e.g.,
/// the code and data segments of the OS kernel, the stack and heap
/// allocated for the OS kernel.
pub struct FrameAllocOptions {
    nframes: usize,
    is_contiguous: bool,
    uninit: bool,
}

impl FrameAllocOptions {
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

    /// Allocates a collection of page frames according to the given options and metadata.
    ///
    /// The metadata function `metadata_fn` is called for each allocated frame to provide metadata.
    pub fn alloc<F, M>(&self, mut metadata_fn: F) -> Result<Vec<Frame<M>>>
    where
        F: FnMut(Paddr) -> M,
        M: FrameMeta,
    {
        let pages = if self.is_contiguous {
            page::allocator::alloc(self.nframes * PAGE_SIZE, |p| {
                FrameMetaBox::new(metadata_fn(p))
            })
            .ok_or(Error::NoMemory)?
        } else {
            page::allocator::alloc_contiguous(self.nframes * PAGE_SIZE, |p| {
                FrameMetaBox::new(metadata_fn(p))
            })
            .ok_or(Error::NoMemory)?
            .into()
        };
        let frames: Vec<_> = pages
            .into_iter()
            .map(|page|
                // SAFETY: The pages are allocated with the metadata type `M`.
                unsafe { Frame::<M>::from_unchecked(page.into()) })
            .collect();
        if !self.uninit {
            for frame in frames.iter() {
                frame.writer().fill(0);
            }
        }

        Ok(frames)
    }

    /// Allocates a single page frame according to the given options and metadata.
    pub fn alloc_single<M: FrameMeta>(&self, metadata: M) -> Result<Frame<M>> {
        if self.nframes != 1 {
            return Err(Error::InvalidArgs);
        }

        let page =
            page::allocator::alloc_single(FrameMetaBox::new(metadata)).ok_or(Error::NoMemory)?;
        // SAFETY: The pages are allocated with the metadata type `M`.
        let frame = unsafe { Frame::<M>::from_unchecked(page.into()) };
        if !self.uninit {
            frame.writer().fill(0);
        }

        Ok(frame)
    }

    /// Allocates a contiguous range of page frames according to the given options.
    ///
    /// The returned [`Segment`] contains at least one page frame.
    pub fn alloc_contiguous<M, F>(&self, mut metadata_fn: F) -> Result<Segment>
    where
        F: FnMut(Paddr) -> M,
        M: FrameMeta,
    {
        // It's no use to checking `self.is_contiguous` here.
        if self.nframes == 0 {
            return Err(Error::InvalidArgs);
        }

        let segment: Segment = page::allocator::alloc_contiguous(self.nframes * PAGE_SIZE, |p| {
            FrameMetaBox::new(metadata_fn(p))
        })
        .ok_or(Error::NoMemory)?
        .into();
        if !self.uninit {
            segment.writer().fill(0);
        }

        Ok(segment)
    }
}

#[cfg(ktest)]
#[ktest]
fn test_alloc_dealloc() {
    // Here we allocate and deallocate frames in random orders to test the allocator.
    // We expect the test to fail if the underlying implementation panics.
    let single_options = FrameAllocOptions::new(1);
    let mut contiguous_options = FrameAllocOptions::new(10);
    contiguous_options.is_contiguous(true);
    let mut remember_vec = Vec::new();
    for _ in 0..10 {
        for i in 0..10 {
            let single_frame = single_options.alloc_single(()).unwrap();
            if i % 3 == 0 {
                remember_vec.push(single_frame);
            }
        }
        let contiguous_segment = contiguous_options.alloc_contiguous(|_| ()).unwrap();
        drop(contiguous_segment);
        remember_vec.pop();
    }
}
