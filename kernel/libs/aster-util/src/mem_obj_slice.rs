// SPDX-License-Identifier: MPL-2.0

// SPDX-License-Identifier: MPL-2.0

//! Provides [`Slice`] for quick duplication and slicing over memory
//! objects, such as [`UFrame`], [`USegment`], [`IoMem`], [`DmaStream`], etc.
//!
//! [`UFrame`]: ostd::mm::UFrame
//! [`USegment`]: ostd::mm::USegment
//! [`IoMem`]: ostd::io::IoMem

use alloc::sync::Arc;
use core::{borrow::Borrow, fmt::Debug, ops::Range};

use ostd::mm::{
    HasDaddr, HasPaddr, HasSize, Infallible, VmReader, VmWriter,
    dma::DmaStream,
    io_util::{HasVmReaderWriter, VmReaderWriterResult},
};

macro_rules! assert_in_range {
    ($range:expr, $size:expr) => {
        assert!(!$range.is_empty(), "The range must not be empty");
        assert!($range.end <= $size, "The range end must be within the size");
    };
}

/// A slice of a memory object.
#[derive(Debug)]
pub struct Slice<MemObj> {
    inner: MemObj,
    offset: Range<usize>,
}

impl<MemObj: HasSize> HasSize for Slice<MemObj> {
    fn size(&self) -> usize {
        self.offset.end - self.offset.start
    }
}

impl<MemObj: HasSize + HasPaddr> HasPaddr for Slice<MemObj> {
    fn paddr(&self) -> ostd::mm::Paddr {
        self.mem_obj().paddr() + self.offset().start
    }
}

impl<MemObj: HasSize + HasDaddr> HasDaddr for Slice<MemObj> {
    fn daddr(&self) -> ostd::mm::Daddr {
        self.mem_obj().daddr() + self.offset().start
    }
}

impl<MemObj: HasSize> Slice<MemObj> {
    /// Creates a new slice from the given offset range.
    ///
    /// The range is relative to the start of the underlying memory object.
    ///
    /// # Panics
    ///
    /// The function panics if
    ///  - the range is empty or negative, or
    ///  - the `range` is not within the range of the underlying memory object.
    pub fn new(mem_obj: MemObj, offset: Range<usize>) -> Self {
        assert_in_range!(offset, mem_obj.size());
        Self {
            inner: mem_obj,
            offset,
        }
    }

    /// Returns the underlying memory object of this slice.
    pub fn mem_obj(&self) -> &MemObj {
        &self.inner
    }

    /// Returns the offset range of this slice within the underlying memory object.
    pub fn offset(&self) -> &Range<usize> {
        &self.offset
    }
}

impl<MemObj: HasSize + Clone> Slice<MemObj> {
    /// Returns a slice of this memory object slice.
    ///
    /// The `range` is relative to the start of the memory object slice, not
    /// the start of the underlying memory object.
    ///
    /// # Panics
    ///
    /// The function panics if
    ///  - the range is empty or negative, or
    ///  - the `range` is not within the range of this memory object slice.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert_in_range!(range, self.size());
        let adj_range = (self.offset().start + range.start)..(self.offset().start + range.end);
        Self::new(self.mem_obj().clone(), adj_range)
    }
}

impl<MemObj: HasSize + HasVmReaderWriter<Types = VmReaderWriterResult>> HasVmReaderWriter
    for Slice<MemObj>
{
    type Types = VmReaderWriterResult;

    fn reader(&self) -> ostd::prelude::Result<VmReader<'_, Infallible>> {
        let mut reader = self.mem_obj().reader()?;
        reader.skip(self.offset().start).limit(self.size());
        Ok(reader)
    }

    fn writer(&self) -> ostd::prelude::Result<VmWriter<'_, Infallible>> {
        let mut writer = self.mem_obj().writer()?;
        writer.skip(self.offset().start).limit(self.size());
        Ok(writer)
    }
}

// A handy implementation for streaming DMA slice.
// TODO: Implement the `sync()` method also for `Slice<DmaStream>`/`Slice<&DmaStream>`,
// and for single-sided ones.
impl<MemObj: HasSize + Borrow<Arc<DmaStream>>> Slice<MemObj> {
    /// Synchronizes the slice of streaming DMA mapping from the device.
    ///
    /// The method will call [`DmaStream::sync_from_device`] with the offset
    /// range of this slice.
    pub fn sync_from_device(&self) -> ostd::prelude::Result<()> {
        self.mem_obj()
            .borrow()
            .sync_from_device(self.offset().clone())
    }

    /// Synchronizes the slice of streaming DMA mapping to the device.
    ///
    /// The method will call [`DmaStream::sync_to_device`] with the offset
    /// range of this slice.
    pub fn sync_to_device(&self) -> ostd::prelude::Result<()> {
        self.mem_obj()
            .borrow()
            .sync_to_device(self.offset().clone())
    }
}
