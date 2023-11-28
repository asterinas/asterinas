use alloc::sync::Arc;
use core::ops::Deref;

use crate::arch::{iommu, mm::PageTableFlags};
use crate::vm::{
    dma::{dma_type, Daddr, DmaType},
    paddr_to_vaddr,
    page_table::KERNEL_PAGE_TABLE,
    HasPaddr, Paddr, VmIo, VmReader, VmSegment, VmWriter, PAGE_SIZE,
};

use super::{check_and_insert_dma_mapping, remove_dma_mapping, DmaError, HasDaddr};

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
    vm_segment: VmSegment,
    start_daddr: Daddr,
    is_cache_coherent: bool,
}

impl DmaCoherent {
    /// Create a coherent DMA mapping backed by `vm_segment`.
    ///
    /// The `is_cache_coherent` argument specifies whether
    /// the target device that the DMA mapping is prepared for
    /// can access the main memory in a CPU cache coherent way
    /// or not.
    ///
    /// The method fails if any part of the given VM segment
    /// already belongs to a DMA mapping.
    pub fn map(vm_segment: VmSegment, is_cache_coherent: bool) -> Result<Self, DmaError> {
        let frame_count = vm_segment.nframes();
        let start_paddr = vm_segment.start_paddr();
        if !check_and_insert_dma_mapping(start_paddr, frame_count) {
            return Err(DmaError::AlreadyMapped);
        }
        if !is_cache_coherent {
            let mut page_table = KERNEL_PAGE_TABLE.get().unwrap().lock();
            for i in 0..frame_count {
                let paddr = start_paddr + (i * PAGE_SIZE);
                let vaddr = paddr_to_vaddr(paddr);
                let flags = page_table.flags(vaddr).unwrap();
                // Safety: the address is in the range of `vm_segment`.
                unsafe {
                    page_table
                        .protect(vaddr, flags.union(PageTableFlags::NO_CACHE))
                        .unwrap();
                }
            }
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
            inner: Arc::new(DmaCoherentInner {
                vm_segment,
                start_daddr,
                is_cache_coherent,
            }),
        })
    }
}

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.inner.start_daddr
    }
}

impl Deref for DmaCoherent {
    type Target = VmSegment;
    fn deref(&self) -> &Self::Target {
        &self.inner.vm_segment
    }
}

impl Drop for DmaCoherentInner {
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
        if !self.is_cache_coherent {
            let mut page_table = KERNEL_PAGE_TABLE.get().unwrap().lock();
            for i in 0..frame_count {
                let paddr = start_paddr + (i * PAGE_SIZE);
                let vaddr = paddr_to_vaddr(paddr);
                let mut flags = page_table.flags(vaddr).unwrap();
                flags.remove(PageTableFlags::NO_CACHE);
                // Safety: the address is in the range of `vm_segment`.
                unsafe {
                    page_table.protect(vaddr, flags).unwrap();
                }
            }
        }
        remove_dma_mapping(start_paddr, frame_count);
    }
}

impl VmIo for DmaCoherent {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> crate::prelude::Result<()> {
        self.inner.vm_segment.read_bytes(offset, buf)
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> crate::prelude::Result<()> {
        self.inner.vm_segment.write_bytes(offset, buf)
    }
}

impl<'a> DmaCoherent {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        self.inner.vm_segment.reader()
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        self.inner.vm_segment.writer()
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        self.inner.vm_segment.start_paddr()
    }
}
