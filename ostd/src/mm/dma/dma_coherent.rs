// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ops::Deref;

use cfg_if::cfg_if;

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError, HasDaddr};
use crate::{
    arch::iommu,
    mm::{
        dma::{dma_type, Daddr, DmaType},
        io_util::{HasVmReaderWriter, VmReaderWriterIdentity},
        kspace::{paddr_to_vaddr, KERNEL_PAGE_TABLE},
        page_prop::CachePolicy,
        HasPaddr, Infallible, Paddr, USegment, VmReader, VmWriter, PAGE_SIZE,
    },
};

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
        use crate::arch::tdx_guest;
    }
}

/// A coherent (or consistent) DMA mapping,
/// which guarantees that the device and the CPU can
/// access the data in parallel.
///
/// The mapping will be destroyed automatically when
/// the object is dropped.
#[derive(Debug, Clone)]
pub struct DmaCoherent {
    inner: Arc<DmaCoherentInner>,
}

#[derive(Debug)]
struct DmaCoherentInner {
    segment: USegment,
    start_daddr: Daddr,
    is_cache_coherent: bool,
}

impl DmaCoherent {
    /// Creates a coherent DMA mapping backed by `segment`.
    ///
    /// The `is_cache_coherent` argument specifies whether
    /// the target device that the DMA mapping is prepared for
    /// can access the main memory in a CPU cache coherent way
    /// or not.
    ///
    /// The method fails if any part of the given `segment`
    /// already belongs to a DMA mapping.
    pub fn map(segment: USegment, is_cache_coherent: bool) -> core::result::Result<Self, DmaError> {
        let start_paddr = segment.start_paddr();
        let frame_count = segment.size() / PAGE_SIZE;

        if !check_and_insert_dma_mapping(start_paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }

        if !is_cache_coherent {
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let vaddr = paddr_to_vaddr(start_paddr);
            let va_range = vaddr..vaddr + (frame_count * PAGE_SIZE);
            // SAFETY: the physical mappings is only used by DMA so protecting it is safe.
            unsafe {
                page_table
                    .protect_flush_tlb(&va_range, |p| p.cache = CachePolicy::Uncacheable)
                    .unwrap();
            }
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
                        tdx_guest::unprotect_gpa_range(start_paddr, frame_count).unwrap();
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
            inner: Arc::new(DmaCoherentInner {
                segment,
                start_daddr,
                is_cache_coherent,
            }),
        })
    }

    /// Returns the number of bytes in the DMA mapping.
    pub fn nbytes(&self) -> usize {
        self.inner.segment.size()
    }
}

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.inner.start_daddr
    }
}

impl Deref for DmaCoherent {
    type Target = USegment;
    fn deref(&self) -> &Self::Target {
        &self.inner.segment
    }
}

impl Drop for DmaCoherentInner {
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
                        tdx_guest::protect_gpa_range(start_paddr, frame_count).unwrap();
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

        if !self.is_cache_coherent {
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let vaddr = paddr_to_vaddr(start_paddr);
            let va_range = vaddr..vaddr + (frame_count * PAGE_SIZE);
            // SAFETY: the physical mappings is only used by DMA so protecting it is safe.
            unsafe {
                page_table
                    .protect_flush_tlb(&va_range, |p| p.cache = CachePolicy::Writeback)
                    .unwrap();
            }
        }

        remove_dma_mapping(start_paddr, frame_count);
    }
}

impl HasVmReaderWriter for DmaCoherent {
    type Types = VmReaderWriterIdentity;

    fn reader(&self) -> VmReader<'_, Infallible> {
        self.inner.segment.reader()
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        self.inner.segment.writer()
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        self.inner.segment.start_paddr()
    }
}
