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

#[if_cfg_ktest]
mod test {
    use super::*;
    use crate::vm::VmAllocOptions;
    use alloc::vec;

    #[ktest]
    fn map_with_coherent_device() {
        let vm_segment = VmAllocOptions::new(1)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_coherent = DmaCoherent::map(vm_segment.clone(), true).unwrap();
        assert!(dma_coherent.paddr() == vm_segment.paddr());
    }

    #[ktest]
    fn map_with_incoherent_device() {
        let vm_segment = VmAllocOptions::new(1)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_coherent = DmaCoherent::map(vm_segment.clone(), false).unwrap();
        assert!(dma_coherent.paddr() == vm_segment.paddr());
        let mut page_table = KERNEL_PAGE_TABLE.get().unwrap().lock();
        assert!(page_table
            .flags(paddr_to_vaddr(vm_segment.paddr()))
            .unwrap()
            .contains(PageTableFlags::NO_CACHE))
    }

    #[ktest]
    fn duplicate_map() {
        let vm_segment_parent = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let vm_segment_child = vm_segment_parent.range(0..1);
        let dma_coherent_parent = DmaCoherent::map(vm_segment_parent, false);
        let dma_coherent_child = DmaCoherent::map(vm_segment_child, false);
        assert!(dma_coherent_child.is_err());
    }

    #[ktest]
    fn read_and_write() {
        let vm_segment = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_coherent = DmaCoherent::map(vm_segment, false).unwrap();

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_coherent.write_bytes(0, &buf_write).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_coherent.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn reader_and_wirter() {
        let vm_segment = VmAllocOptions::new(2)
            .is_contiguous(true)
            .alloc_contiguous()
            .unwrap();
        let dma_coherent = DmaCoherent::map(vm_segment, false).unwrap();

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_coherent.writer();
        writer.write(&mut buf_write.as_slice().into());
        writer.write(&mut buf_write.as_slice().into());

        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        let mut reader = dma_coherent.reader();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }
}
