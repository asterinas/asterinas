// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ops::Deref;

use cfg_if::cfg_if;

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError, HasDaddr};
use crate::{
    arch::iommu,
    if_tdx_enabled,
    mm::{
        dma::{dma_type, Daddr, DmaType},
        io::VmIoOnce,
        kspace::{paddr_to_vaddr, KERNEL_PAGE_TABLE},
        page_prop::CachePolicy,
        HasPaddr, Infallible, Paddr, PodOnce, USegment, UntypedMem, VmIo, VmReader, VmWriter,
        PAGE_SIZE,
    },
    prelude::*,
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
        let frame_count = segment.size() / PAGE_SIZE;
        let start_paddr = segment.start_paddr();
        if !check_and_insert_dma_mapping(start_paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }
        // Ensure that the addresses used later will not overflow
        start_paddr.checked_add(frame_count * PAGE_SIZE).unwrap();
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
                if_tdx_enabled!({
                    #[cfg(target_arch = "x86_64")]
                    // SAFETY:
                    // This is safe because we are ensuring that the physical address range specified by `start_paddr` and `frame_count` is valid before these operations.
                    // The `check_and_insert_dma_mapping` function checks if the physical address range is already mapped.
                    // We are also ensuring that we are only modifying the page table entries corresponding to the physical address range specified by `start_paddr` and `frame_count`.
                    // Therefore, we are not causing any undefined behavior or violating any of the requirements of the 'unprotect_gpa_range' function.
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
        let frame_count = self.segment.size() / PAGE_SIZE;
        let start_paddr = self.segment.start_paddr();
        // Ensure that the addresses used later will not overflow
        start_paddr.checked_add(frame_count * PAGE_SIZE).unwrap();
        match dma_type() {
            DmaType::Direct => {
                if_tdx_enabled!({
                    #[cfg(target_arch = "x86_64")]
                    // SAFETY:
                    // This is safe because we are ensuring that the physical address range specified by `start_paddr` and `frame_count` is valid before these operations.
                    // The `start_paddr()` ensures the `start_paddr` is page-aligned.
                    // We are also ensuring that we are only modifying the page table entries corresponding to the physical address range specified by `start_paddr` and `frame_count`.
                    // Therefore, we are not causing any undefined behavior or violating any of the requirements of the `protect_gpa_range` function.
                    unsafe {
                        tdx_guest::protect_gpa_range(start_paddr, frame_count).unwrap();
                    }
                });
            }
            DmaType::Iommu => {
                for i in 0..frame_count {
                    let paddr = start_paddr + (i * PAGE_SIZE);
                    iommu::unmap(paddr).unwrap();
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

impl VmIo for DmaCoherent {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        self.inner.segment.read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        self.inner.segment.write(offset, reader)
    }
}

impl VmIoOnce for DmaCoherent {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        self.inner.segment.reader().skip(offset).read_once()
    }

    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.inner.segment.writer().skip(offset).write_once(new_val)
    }
}

impl<'a> DmaCoherent {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a, Infallible> {
        self.inner.segment.reader()
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a, Infallible> {
        self.inner.segment.writer()
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        self.inner.segment.start_paddr()
    }
}
