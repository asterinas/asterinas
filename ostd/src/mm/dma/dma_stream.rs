// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ops::Range;

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError, HasDaddr};
use crate::{
    arch::iommu,
    error::Error,
    mm::{
        dma::{dma_type, Daddr, DmaType},
        HasPaddr, Infallible, Paddr, USegment, UntypedMem, VmIo, VmReader, VmWriter, PAGE_SIZE,
    },
};

/// A streaming DMA mapping. Users must synchronize data
/// before reading or after writing to ensure consistency.
///
/// The mapping is automatically destroyed when this object
/// is dropped.
#[derive(Debug, Clone)]
pub struct DmaStream {
    inner: Arc<DmaStreamInner>,
}

#[derive(Debug)]
struct DmaStreamInner {
    segment: USegment,
    start_daddr: Daddr,
    /// TODO: remove this field when on x86.
    #[expect(unused)]
    is_cache_coherent: bool,
    direction: DmaDirection,
}

/// `DmaDirection` limits the data flow direction of [`DmaStream`] and
/// prevents users from reading and writing to [`DmaStream`] unexpectedly.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DmaDirection {
    /// Data flows to the device
    ToDevice,
    /// Data flows from the device
    FromDevice,
    /// Data flows both from and to the device
    Bidirectional,
}

impl DmaStream {
    /// Establishes DMA stream mapping for a given [`USegment`].
    ///
    /// The method fails if the segment already belongs to a DMA mapping.
    pub fn map(
        segment: USegment,
        direction: DmaDirection,
        is_cache_coherent: bool,
    ) -> Result<Self, DmaError> {
        let start_paddr = segment.start_paddr();
        let frame_count = segment.size() / PAGE_SIZE;

        if !check_and_insert_dma_mapping(start_paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }

        let start_daddr = match dma_type() {
            DmaType::Direct => {
                #[cfg(target_arch = "x86_64")]
                crate::arch::if_tdx_enabled!({
                    // SAFETY:
                    //  - The address of a `USegment` is always page aligned.
                    //  - A `USegment` always points to normal physical memory, so the address
                    //    range falls in the GPA limit.
                    //  - A `USegment` always points to normal physical memory, so all the pages
                    //    are contained in the linear mapping.
                    //  - The pages belong to a `USegment`, so they're all untyped memory.
                    unsafe {
                        crate::arch::tdx_guest::unprotect_gpa_range(start_paddr, frame_count)
                            .unwrap();
                    }
                });
                start_paddr as Daddr
            }
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = start_paddr + (i * PAGE_SIZE);
                    // SAFETY: the `paddr` is restricted by the `start_paddr` and `frame_count` of the `segment`.
                    unsafe {
                        iommu::map(paddr as Daddr, paddr).unwrap();
                    }
                }
                start_paddr as Daddr
            }
        };

        Ok(Self {
            inner: Arc::new(DmaStreamInner {
                segment,
                start_daddr,
                is_cache_coherent,
                direction,
            }),
        })
    }

    /// Gets the underlying [`USegment`].
    ///
    /// Usually, the CPU side should not access the memory
    /// after the DMA mapping is established because
    /// there is a chance that the device is updating
    /// the memory. Do this at your own risk.
    pub fn segment(&self) -> &USegment {
        &self.inner.segment
    }

    /// Returns the number of frames.
    pub fn nframes(&self) -> usize {
        self.inner.segment.size() / PAGE_SIZE
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.inner.segment.size()
    }

    /// Returns the DMA direction.
    pub fn direction(&self) -> DmaDirection {
        self.inner.direction
    }

    /// Synchronizes the streaming DMA mapping with the device.
    ///
    /// This method should be called under one of the two conditions:
    /// 1. The data of the stream DMA mapping has been updated by the device side.
    ///    The CPU side needs to call the `sync` method before reading data (e.g., using [`read_bytes`]).
    /// 2. The data of the stream DMA mapping has been updated by the CPU side
    ///    (e.g., using [`write_bytes`]).
    ///    Before the CPU side notifies the device side to read, it must call the `sync` method first.
    ///
    /// [`read_bytes`]: Self::read_bytes
    /// [`write_bytes`]: Self::write_bytes
    pub fn sync(&self, _byte_range: Range<usize>) -> Result<(), Error> {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")]{
                // The streaming DMA mapping in x86_64 is cache coherent, and does not require synchronization.
                // Reference: <https://lwn.net/Articles/855328/>, <https://lwn.net/Articles/2265/>
                Ok(())
            } else {
                if _byte_range.end > self.nbytes() {
                    return Err(Error::InvalidArgs);
                }
                if self.inner.is_cache_coherent {
                    return Ok(());
                }
                let start_va = crate::mm::paddr_to_vaddr(self.inner.segment.start_paddr()) as *const u8;
                // TODO: Query the CPU for the cache line size via CPUID, we use 64 bytes as the cache line size here.
                for i in _byte_range.step_by(64) {
                    // TODO: Call the cache line flush command in the corresponding architecture.
                    todo!()
                }
                Ok(())
            }
        }
    }
}

impl HasDaddr for DmaStream {
    fn daddr(&self) -> Daddr {
        self.inner.start_daddr
    }
}

impl Drop for DmaStreamInner {
    fn drop(&mut self) {
        let start_paddr = self.segment.start_paddr();
        let frame_count = self.segment.size() / PAGE_SIZE;

        match dma_type() {
            DmaType::Direct => {
                #[cfg(target_arch = "x86_64")]
                crate::arch::if_tdx_enabled!({
                    // SAFETY:
                    //  - The address of a `USegment` is always page aligned.
                    //  - A `USegment` always points to normal physical memory, so the address
                    //    range falls in the GPA limit.
                    //  - A `USegment` always points to normal physical memory, so all the pages
                    //    are contained in the linear mapping.
                    //  - The pages belong to a `USegment`, so they're all untyped memory.
                    unsafe {
                        crate::arch::tdx_guest::protect_gpa_range(start_paddr, frame_count)
                            .unwrap();
                    }
                });
            }
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = start_paddr + (i * PAGE_SIZE);
                    iommu::unmap(paddr as Daddr).unwrap();
                    // FIXME: After dropping it could be reused. IOTLB needs to be flushed.
                }
            }
        }

        remove_dma_mapping(start_paddr, frame_count);
    }
}

impl VmIo for DmaStream {
    /// Reads data into the buffer.
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), Error> {
        if self.inner.direction == DmaDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.segment.read(offset, writer)
    }

    /// Writes data from the buffer.
    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), Error> {
        if self.inner.direction == DmaDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.segment.write(offset, reader)
    }
}

impl<'a> DmaStream {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> Result<VmReader<'a, Infallible>, Error> {
        if self.inner.direction == DmaDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.segment.reader())
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> Result<VmWriter<'a, Infallible>, Error> {
        if self.inner.direction == DmaDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.segment.writer())
    }
}

impl HasPaddr for DmaStream {
    fn paddr(&self) -> Paddr {
        self.inner.segment.start_paddr()
    }
}

impl AsRef<DmaStream> for DmaStream {
    fn as_ref(&self) -> &DmaStream {
        self
    }
}

/// A slice of streaming DMA mapping.
#[derive(Debug)]
pub struct DmaStreamSlice<Dma> {
    stream: Dma,
    offset: usize,
    len: usize,
}

impl<Dma: AsRef<DmaStream>> DmaStreamSlice<Dma> {
    /// Constructs a `DmaStreamSlice` from the [`DmaStream`].
    ///
    /// # Panics
    ///
    /// If the `offset` is greater than or equal to the length of the stream,
    /// this method will panic.
    /// If the `offset + len` is greater than the length of the stream,
    /// this method will panic.
    pub fn new(stream: Dma, offset: usize, len: usize) -> Self {
        assert!(offset < stream.as_ref().nbytes());
        assert!(offset + len <= stream.as_ref().nbytes());

        Self {
            stream,
            offset,
            len,
        }
    }

    /// Returns the underlying `DmaStream`.
    pub fn stream(&self) -> &DmaStream {
        self.stream.as_ref()
    }

    /// Returns the offset of the slice.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.len
    }

    /// Synchronizes the slice of streaming DMA mapping with the device.
    pub fn sync(&self) -> Result<(), Error> {
        self.stream
            .as_ref()
            .sync(self.offset..self.offset + self.len)
    }

    /// Returns a reader to read data from it.
    pub fn reader(&self) -> Result<VmReader<Infallible>, Error> {
        let mut stream_reader = self.stream.as_ref().reader()?;
        stream_reader.skip(self.offset).limit(self.len);
        Ok(stream_reader)
    }

    /// Returns a writer to write data into it.
    pub fn writer(&self) -> Result<VmWriter<Infallible>, Error> {
        let mut stream_writer = self.stream.as_ref().writer()?;
        stream_writer.skip(self.offset).limit(self.len);
        Ok(stream_writer)
    }
}

impl<Dma: AsRef<DmaStream> + Send + Sync> VmIo for DmaStreamSlice<Dma> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), Error> {
        if writer.avail() + offset > self.len {
            return Err(Error::InvalidArgs);
        }
        self.stream.as_ref().read(self.offset + offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), Error> {
        if reader.remain() + offset > self.len {
            return Err(Error::InvalidArgs);
        }
        self.stream.as_ref().write(self.offset + offset, reader)
    }
}

impl<Dma: AsRef<DmaStream>> HasDaddr for DmaStreamSlice<Dma> {
    fn daddr(&self) -> Daddr {
        self.stream.as_ref().daddr() + self.offset
    }
}

impl<Dma: AsRef<DmaStream>> HasPaddr for DmaStreamSlice<Dma> {
    fn paddr(&self) -> Paddr {
        self.stream.as_ref().paddr() + self.offset
    }
}

impl<Dma: AsRef<DmaStream> + Clone> Clone for DmaStreamSlice<Dma> {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.clone(),
            offset: self.offset,
            len: self.len,
        }
    }
}
