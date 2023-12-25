use alloc::sync::Arc;
use core::arch::x86_64::_mm_clflush;
use core::ops::Range;

use crate::arch::iommu;
use crate::error::Error;
use crate::vm::{
    dma::{dma_type, Daddr, DmaType},
    HasPaddr, Paddr, VmSegment, PAGE_SIZE,
};
use crate::vm::{VmIo, VmReader, VmWriter};

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError, HasDaddr};

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
    vm_segment: VmSegment,
    start_daddr: Daddr,
    is_cache_coherent: bool,
    direction: DmaDirection,
}

/// `DmaDirection` limits the data flow direction of `DmaStream` and
/// prevents users from reading and writing to `DmaStream` unexpectedly.
#[derive(Debug, PartialEq, Clone)]
pub enum DmaDirection {
    ToDevice,
    FromDevice,
    Bidirectional,
}

impl DmaStream {
    /// Establish DMA stream mapping for a given `VmSegment`.
    ///
    /// The method fails if the segment already belongs to a DMA mapping.
    pub fn map(
        vm_segment: VmSegment,
        direction: DmaDirection,
        is_cache_coherent: bool,
    ) -> Result<Self, DmaError> {
        let frame_count = vm_segment.nframes();
        let start_paddr = vm_segment.start_paddr();
        if !check_and_insert_dma_mapping(start_paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }
        let start_daddr = match dma_type() {
            DmaType::Direct => start_paddr as Daddr,
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = start_paddr + (i * PAGE_SIZE);
                    // Safety: the `paddr` is restricted by the `start_paddr` and `frame_count` of the `vm_segment`.
                    unsafe {
                        iommu::map(paddr as Daddr, paddr).unwrap();
                    }
                }
                start_paddr as Daddr
            }
            DmaType::Tdx => {
                todo!()
            }
        };

        Ok(Self {
            inner: Arc::new(DmaStreamInner {
                vm_segment,
                start_daddr,
                is_cache_coherent,
                direction,
            }),
        })
    }

    /// Get the underlying VM segment.
    ///
    /// Usually, the CPU side should not access the memory
    /// after the DMA mapping is established because
    /// there is a chance that the device is updating
    /// the memory. Do this at your own risk.
    pub fn vm_segment(&self) -> &VmSegment {
        &self.inner.vm_segment
    }

    pub fn nbytes(&self) -> usize {
        self.inner.vm_segment.nbytes()
    }

    /// Synchronize the streaming DMA mapping with the device.
    ///
    /// This method should be called under one of the two conditions:
    /// 1. The data of the stream DMA mapping has been updated by the device side.
    /// The CPU side needs to call the `sync` method before reading data (e.g., using `read_bytes`).
    /// 2. The data of the stream DMA mapping has been updated by the CPU side
    /// (e.g., using `write_bytes`).
    /// Before the CPU side notifies the device side to read, it must call the `sync` method first.
    pub fn sync(&self, byte_range: Range<usize>) -> Result<(), Error> {
        if byte_range.end > self.nbytes() {
            return Err(Error::InvalidArgs);
        }
        if self.inner.is_cache_coherent {
            return Ok(());
        }
        if dma_type() == DmaType::Tdx {
            // copy pages.
            todo!("support dma for tdx")
        } else {
            let start_va = self.inner.vm_segment.as_ptr();
            // TODO: Query the CPU for the cache line size via CPUID, we use 64 bytes as the cache line size here.
            for i in byte_range.step_by(64) {
                // Safety: the addresses is limited by a valid `byte_range`.
                unsafe {
                    _mm_clflush(start_va.wrapping_add(i));
                }
            }
            Ok(())
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
        let frame_count = self.vm_segment.nframes();
        let start_paddr = self.vm_segment.start_paddr();
        match dma_type() {
            DmaType::Direct => {}
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = start_paddr + (i * PAGE_SIZE);
                    iommu::unmap(paddr).unwrap();
                }
            }
            DmaType::Tdx => {
                todo!();
            }
        }
        remove_dma_mapping(start_paddr, frame_count);
    }
}

impl VmIo for DmaStream {
    /// Read data into the buffer.
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<(), Error> {
        if self.inner.direction == DmaDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.vm_segment.read_bytes(offset, buf)
    }

    /// Write data from the buffer.
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<(), Error> {
        if self.inner.direction == DmaDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.vm_segment.write_bytes(offset, buf)
    }
}

impl<'a> DmaStream {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> Result<VmReader<'a>, Error> {
        if self.inner.direction == DmaDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.vm_segment.reader())
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> Result<VmWriter<'a>, Error> {
        if self.inner.direction == DmaDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.vm_segment.writer())
    }
}

impl HasPaddr for DmaStream {
    fn paddr(&self) -> Paddr {
        self.inner.vm_segment.start_paddr()
    }
}

#[if_cfg_ktest]
mod test {
    use super::*;
    use crate::vm::VmAllocOptions;
    use alloc::vec;

    #[ktest]
    fn streaming_map() {
        let vm_segment = VmAllocOptions::new(1)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_stream =
            DmaStream::map(vm_segment.clone(), DmaDirection::Bidirectional, true).unwrap();
        assert!(dma_stream.paddr() == vm_segment.paddr());
    }

    #[ktest]
    fn duplicate_map() {
        let vm_segment_parent = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let vm_segment_child = vm_segment_parent.range(0..1);
        let dma_stream_parent =
            DmaStream::map(vm_segment_parent, DmaDirection::Bidirectional, false);
        let dma_stream_child = DmaStream::map(vm_segment_child, DmaDirection::Bidirectional, false);
        assert!(dma_stream_child.is_err());
    }

    #[ktest]
    fn read_and_write() {
        let vm_segment = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_stream = DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap();

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_stream.write_bytes(0, &buf_write).unwrap();
        dma_stream.sync(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn reader_and_wirter() {
        let vm_segment = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_stream = DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap();

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_stream.writer().unwrap();
        writer.write(&mut buf_write.as_slice().into());
        writer.write(&mut buf_write.as_slice().into());
        dma_stream.sync(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        let mut reader = dma_stream.reader().unwrap();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }
}
