// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use cfg_if::cfg_if;

use super::{DmaError, check_and_insert_dma_mapping, remove_dma_mapping};
use crate::{
    arch::iommu,
    mm::{
        HasDaddr, HasPaddr, HasSize, Infallible, PAGE_SIZE, Paddr, USegment, VmReader, VmWriter,
        dma::{Daddr, DmaType, dma_type},
        io_util::{HasVmReaderWriter, VmReaderWriterIdentity},
        kspace::{KERNEL_PAGE_TABLE, paddr_to_vaddr},
        page_prop::CachePolicy,
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
#[derive(Debug)]
pub struct DmaCoherent {
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
        let paddr = segment.paddr();
        let frame_count = segment.size() / PAGE_SIZE;

        if !check_and_insert_dma_mapping(paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }

        if !is_cache_coherent {
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let vaddr = paddr_to_vaddr(paddr);
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
                    // TODO: Prevent Iago attack: DMA buffer unprotection creates VMM data access boundary:
                    // unprotect_gpa_range() exposes DMA buffers to untrusted VMM as shared memory for IO.
                    // Primary Security Risks (TDX threat model scope):
                    // - **Kernel Data Integrity Violation**: VMM may modify DMA buffer contents,
                    //    corrupting data that kernel expects to be authentic from devices. This affects:
                    //    - VirtQueue descriptors, available/used rings (control flow corruption)
                    //    - Network packet data (data integrity compromise)
                    //    - Storage I/O buffers (file system data corruption)
                    // - **Privilege Escalation Risk**: Malicious DMA buffer content may exploit
                    //    kernel VirtIO driver vulnerabilities through:
                    //    - Crafted VirtQueue ring entries causing buffer overflows
                    //    - Invalid descriptor chains leading to memory corruption
                    //    - Malformed device responses triggering driver parsing errors
                    // VirtQueue-Specific Attack Vectors:
                    // - **Descriptor manipulation**: VMM modifies buffer addresses/lengths in descriptors
                    // - **Ring tampering**: VMM corrupts available/used ring indices and flags
                    // - **Buffer content injection**: VMM injects malicious data in I/O buffers
                    // Consider implementing:
                    // - Validate VirtQueue ring structures before processing
                    // - Bounds-check all buffer addresses and lengths from device
                    // - Sanitize device-provided data before kernel processing
                    unsafe {
                        tdx_guest::unprotect_gpa_range(paddr, frame_count).unwrap();
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
            segment,
            start_daddr,
            is_cache_coherent,
        })
    }
}

impl Deref for DmaCoherent {
    type Target = USegment;
    fn deref(&self) -> &Self::Target {
        &self.segment
    }
}

impl Drop for DmaCoherent {
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
                        tdx_guest::protect_gpa_range(paddr, frame_count).unwrap();
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

        if !self.is_cache_coherent {
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let vaddr = paddr_to_vaddr(paddr);
            let va_range = vaddr..vaddr + (frame_count * PAGE_SIZE);
            // SAFETY: the physical mappings is only used by DMA so protecting it is safe.
            unsafe {
                page_table
                    .protect_flush_tlb(&va_range, |p| p.cache = CachePolicy::Writeback)
                    .unwrap();
            }
        }

        remove_dma_mapping(paddr, frame_count);
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        self.segment.paddr()
    }
}

impl HasSize for DmaCoherent {
    fn size(&self) -> usize {
        self.segment.size()
    }
}

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.start_daddr
    }
}

impl HasVmReaderWriter for DmaCoherent {
    type Types = VmReaderWriterIdentity;

    fn reader(&self) -> VmReader<'_, Infallible> {
        self.segment.reader()
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        self.segment.writer()
    }
}
