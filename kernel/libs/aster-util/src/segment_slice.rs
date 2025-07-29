// SPDX-License-Identifier: MPL-2.0

// SPDX-License-Identifier: MPL-2.0

//! Provides [`SegmentSlice`] for quick duplication and slicing over [`USegment`].

use alloc::sync::Arc;
use core::ops::Range;

use ostd::mm::{
    io_util::{HasVmReaderWriter, VmReaderWriterIdentity},
    Infallible, Paddr, UFrame, USegment, VmReader, VmWriter, PAGE_SIZE,
};

/// A reference to a slice of a [`USegment`].
///
/// Cloning a [`SegmentSlice`] is cheap, as it only increments one reference
/// count. While cloning a [`USegment`] will increment the reference count of
/// many underlying pages.
///
/// The downside is that the [`SegmentSlice`] requires heap allocation. Also,
/// if any [`SegmentSlice`] of the original [`USegment`] is alive, all pages in
/// the original [`USegment`], including the pages that are not referenced, will
/// not be freed.
#[derive(Debug, Clone)]
pub struct SegmentSlice {
    inner: Arc<USegment>,
    range: Range<usize>,
}

impl SegmentSlice {
    /// Returns a part of the `USegment`.
    ///
    /// # Panics
    ///
    /// If `range` is not within the range of this `USegment`,
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
}

impl HasVmReaderWriter for SegmentSlice {
    type Types = VmReaderWriterIdentity;

    fn reader(&self) -> VmReader<'_, Infallible> {
        let mut reader = self.inner.reader();
        reader
            .skip(self.start_paddr() - self.inner.start_paddr())
            .limit(self.nbytes());
        reader
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        let mut writer = self.inner.writer();
        writer
            .skip(self.start_paddr() - self.inner.start_paddr())
            .limit(self.nbytes());
        writer
    }
}

impl From<USegment> for SegmentSlice {
    fn from(segment: USegment) -> Self {
        let range = 0..segment.size() / PAGE_SIZE;
        Self {
            inner: Arc::new(segment),
            range,
        }
    }
}

impl From<SegmentSlice> for USegment {
    fn from(slice: SegmentSlice) -> Self {
        let start = slice.range.start * PAGE_SIZE;
        let end = slice.range.end * PAGE_SIZE;
        slice.inner.slice(&(start..end))
    }
}

impl From<UFrame> for SegmentSlice {
    fn from(frame: UFrame) -> Self {
        SegmentSlice::from(USegment::from(frame))
    }
}
