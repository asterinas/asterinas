// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(
    any(target_arch = "riscv64", target_arch = "loongarch64"),
    allow(unfulfilled_lint_expectations)
)]

use alloc::sync::Arc;
use core::ops::Range;

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError};
use crate::{
    arch::iommu,
    error::Error,
    mm::{
        dma::{dma_type, Daddr, DmaType},
        io_util::{HasVmReaderWriter, VmReaderWriterResult},
        HasDaddr, HasPaddr, HasSize, Infallible, Paddr, USegment, VmReader, VmWriter, PAGE_SIZE,
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
        let paddr = segment.paddr();
        let frame_count = segment.size() / PAGE_SIZE;

        if !check_and_insert_dma_mapping(paddr, frame_count) {
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
                        crate::arch::tdx_guest::unprotect_gpa_range(paddr, frame_count).unwrap();
                    }
                });
                paddr as Daddr
            }
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = paddr + (i * PAGE_SIZE);
                    // SAFETY: the `paddr` is restricted by the `paddr` and `frame_count` of the `segment`.
                    unsafe {
                        iommu::map(paddr as Daddr, paddr).unwrap();
                    }
                }
                paddr as Daddr
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
    /// [`read_bytes`]: crate::mm::VmIo::read_bytes
    /// [`write_bytes`]: crate::mm::VmIo::write_bytes
    pub fn sync(&self, _byte_range: Range<usize>) -> Result<(), Error> {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")]{
                // The streaming DMA mapping in x86_64 is cache coherent, and does not require synchronization.
                // Reference: <https://lwn.net/Articles/855328/>, <https://lwn.net/Articles/2265/>
                Ok(())
            } else {
                if _byte_range.end > self.size() {
                    return Err(Error::InvalidArgs);
                }
                if self.inner.is_cache_coherent {
                    return Ok(());
                }
                let _start_va = crate::mm::paddr_to_vaddr(self.inner.segment.paddr()) as *const u8;
                // TODO: Query the CPU for the cache line size via CPUID, we use 64 bytes as the cache line size here.
                for _i in _byte_range.step_by(64) {
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
        let paddr = self.segment.paddr();
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
                        crate::arch::tdx_guest::protect_gpa_range(paddr, frame_count).unwrap();
                    }
                });
            }
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = paddr + (i * PAGE_SIZE);
                    iommu::unmap(paddr as Daddr).unwrap();
                    // FIXME: After dropping it could be reused. IOTLB needs to be flushed.
                }
            }
        }

        remove_dma_mapping(paddr, frame_count);
    }
}

impl HasVmReaderWriter for DmaStream {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if self.inner.direction == DmaDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.segment.reader())
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if self.inner.direction == DmaDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        Ok(self.inner.segment.writer())
    }
}

impl HasPaddr for DmaStream {
    fn paddr(&self) -> Paddr {
        self.inner.segment.paddr()
    }
}

impl HasSize for DmaStream {
    fn size(&self) -> usize {
        self.inner.segment.size()
    }
}
