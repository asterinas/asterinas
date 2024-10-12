// SPDX-License-Identifier: MPL-2.0

// SPDX-License-Identifier: MPL-2.0

//! Provides [`SegmentSlice`] for quick duplication and slicing over [`Segment`].

use alloc::sync::Arc;
use core::ops::Range;

use ostd::{
    mm::{
        FallibleVmRead, FallibleVmWrite, Frame, Infallible, Paddr, Segment, VmIo, VmReader,
        VmWriter, PAGE_SIZE,
    },
    Error, Result,
};

/// A reference to a slice of a [`Segment`].
///
/// Cloning a [`SegmentSlice`] is cheap, as it only increments one reference
/// count. While cloning a [`Segment`] will increment the reference count of
/// many underlying pages.
///
/// The downside is that the [`SegmentSlice`] requires heap allocation. Also,
/// if any [`SegmentSlice`] of the original [`Segment`] is alive, all pages in
/// the original [`Segment`], including the pages that are not referenced, will
/// not be freed.
#[derive(Debug, Clone)]
pub struct SegmentSlice {
    inner: Arc<Segment>,
    range: Range<usize>,
}

impl SegmentSlice {
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

    /// Gets a reader for the slice.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.inner
            .reader()
            .skip(self.start_paddr() - self.inner.start_paddr())
            .limit(self.nbytes())
    }

    /// Gets a writer for the slice.
    pub fn writer(&self) -> VmWriter<'_, Infallible> {
        self.inner
            .writer()
            .skip(self.start_paddr() - self.inner.start_paddr())
            .limit(self.nbytes())
    }

    fn start_frame_index(&self) -> usize {
        self.inner.start_paddr() / PAGE_SIZE + self.range.start
    }
}

impl VmIo for SegmentSlice {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail();
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(read_len).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self
            .reader()
            .skip(offset)
            .read_fallible(writer)
            .map_err(|(e, _)| e)?;
        debug_assert!(len == read_len);
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain();
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(reader.remain()).ok_or(Error::Overflow)?;
        if max_offset > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        let len = self
            .writer()
            .skip(offset)
            .write_fallible(reader)
            .map_err(|(e, _)| e)?;
        debug_assert!(len == write_len);
        Ok(())
    }
}

impl From<Segment> for SegmentSlice {
    fn from(segment: Segment) -> Self {
        let range = 0..segment.nbytes() / PAGE_SIZE;
        Self {
            inner: Arc::new(segment),
            range,
        }
    }
}

impl From<SegmentSlice> for Segment {
    fn from(slice: SegmentSlice) -> Self {
        let start = slice.range.start * PAGE_SIZE;
        let end = slice.range.end * PAGE_SIZE;
        slice.inner.slice(&(start..end))
    }
}

impl From<Frame> for SegmentSlice {
    fn from(frame: Frame) -> Self {
        SegmentSlice::from(Segment::from(frame))
    }
}
