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
    zeroed: bool,
}

impl Default for FrameAllocOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAllocOptions {
    /// Creates new options for allocating frames.
    pub fn new() -> Self {
        Self { zeroed: true }
    }

    /// Sets whether the allocated frames should be initialized to zero.
    ///
    /// By default this is `true`. Note that setting this to `false` may
    /// improve performance, but it definitely introduces security
    /// vulnerabilities by leaking sensitive information. The caller is
    /// responsible for writing to all the bits of the allocated frames
    /// before sharing them with other components.
    pub fn zeroed(&mut self, zeroed: bool) -> &mut Self {
        self.zeroed = zeroed;
        self
    }

    /// Allocates a single page frame with provided metadata.
    pub fn alloc_single<M: FrameMeta>(&self, metadata: M) -> Result<Frame<M>> {
        let page =
            page::allocator::alloc_single(FrameMetaBox::new(metadata)).ok_or(Error::NoMemory)?;
        // SAFETY: The pages are allocated with the metadata type `M`.
        let frame = unsafe { Frame::<M>::from_unchecked(page.into()) };
        if self.zeroed {
            frame.writer().fill(0);
        }

        Ok(frame)
    }

    /// Allocates a contiguous range of page frames with provided metadata.
    ///
    /// The metadata is initialized with the caller-provided closure
    /// `metadata_fn`. The closure receives the physical address of the
    /// page and returns the metadata for the page.
    pub fn alloc_contiguous<M, F>(&self, nframes: usize, mut metadata_fn: F) -> Result<Segment>
    where
        F: FnMut(Paddr) -> M,
        M: FrameMeta,
    {
        let segment: Segment = page::allocator::alloc_contiguous(nframes * PAGE_SIZE, |p| {
            FrameMetaBox::new(metadata_fn(p))
        })
        .ok_or(Error::NoMemory)?
        .into();
        if self.zeroed {
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
    let options = FrameAllocOptions::new();
    let mut remember_vec = Vec::new();
    for _ in 0..10 {
        for i in 0..10 {
            let single_frame = options.alloc_single(()).unwrap();
            if i % 3 == 0 {
                remember_vec.push(single_frame);
            }
        }
        let contiguous_segment = options.alloc_contiguous(10, |_| ()).unwrap();
        drop(contiguous_segment);
        remember_vec.pop();
    }
}
