// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{
    Result,
    mm::{
        Daddr, HasDaddr, HasSize, Infallible, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
};

use crate::DmaSegment;

/// A DMA buffer, which is either directly allocated/mapped or comes from a pool.
#[derive(Debug)]
pub enum DmaBuffer<D: DmaDirection> {
    /// A DMA buffer that is directly allocated/mapped.
    Direct(DmaStream<D>),
    /// A DMA buffer that comes from a pool.
    Pooled(DmaSegment<D>),
}

impl<D: DmaDirection> DmaBuffer<D> {
    /// Synchronizes the streaming DMA mapping data from the device.
    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Direct(stream) => stream.sync_from_device(byte_range),
            Self::Pooled(segment) => segment.sync_from_device(byte_range),
        }
    }

    /// Synchronizes the streaming DMA mapping data to the device.
    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Direct(stream) => stream.sync_to_device(byte_range),
            Self::Pooled(segment) => segment.sync_to_device(byte_range),
        }
    }
}

impl<D: DmaDirection> HasDaddr for DmaBuffer<D> {
    fn daddr(&self) -> Daddr {
        match self {
            Self::Direct(stream) => stream.daddr(),
            Self::Pooled(segment) => segment.daddr(),
        }
    }
}

impl<D: DmaDirection> HasSize for DmaBuffer<D> {
    fn size(&self) -> usize {
        match self {
            Self::Direct(stream) => stream.size(),
            Self::Pooled(segment) => segment.size(),
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaBuffer<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        match self {
            Self::Direct(stream) => stream.reader(),
            Self::Pooled(segment) => segment.reader(),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        match self {
            Self::Direct(stream) => stream.writer(),
            Self::Pooled(segment) => segment.writer(),
        }
    }
}
