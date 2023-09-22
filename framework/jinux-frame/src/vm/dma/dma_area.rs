use core::ops::Deref;

use alloc::sync::Weak;

use super::{dma_area_pool::DmaAreaPool, dma_type, sync_frame_vec, DmaType};
use crate::{
    arch::iommu::{self, iova::alloc_iova_continuous},
    bus::pci::PciDeviceLocation,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
    vm::{VmFrameVec, VmIo},
    Result,
};

/// `DmaArea` represents a set of incoherent DMA mappings. It does not disable cache on
/// mapped physical pages, but instead synchronizes data through software.
///
/// To ensure coherence, users are unable to directly access data within `DmaArea`,
/// and must instead construct `DmaAreaReader` and `DmaAreaWrite` through `read()`
/// and `write()` to access data. `DmaAreaReader` and `DmaAreaWrite` will guarantee
/// data synchronization during construction or destruction.
///
/// Moreover, DmaArea is commonly used as various buffers in device drivers with a frequency
/// of allocation and deallocation. In order to ensure allocation speed, it can be
/// constructed from `DmaAreaPool`. If the user's requirement is temporary, `from_vm_frame_vec()`
/// can also be used.
///
/// # Example
///
/// ```rust
/// use jinux_frame::vm::{DmaArea, DmaAreaPool}
///
/// // alloc a dma_area with one page from dma_area_pool
/// let dma_area_pool = DmaAreaPool::new();
/// let dma_area = dma_area_pool
///
/// let dma_area_pool = DmaAreaPool::new().unwrap();
/// // A dma_area with one page
/// let dma_area = dma_area_pool.alloc().unwrap();
/// // Write data by writer
/// let writer = dma_area.write();
/// let write_buffer = vec![1,2,3,4];
/// writer.write_bytes(0, &write_buffer).unwrap();
/// drop(writer);
/// // Read data by reader
/// let reader = dma_area.read();
/// let mut read_buffer = vec![0u8; 4];
/// reader.read_bytes(0, &mut read_buffer).unwrap();
/// assert_eq!(read_buffer, write_buffer);
///
/// // alloc a dma_area with continuous pages from dma_area_pool
/// let dma_area = dma_area_pool.alloc_continuous(4).unwrap();
/// // Write data by writer
/// let writer = dma_area.write();
/// let write_buffer = vec![1u8; 2*PAGE_SIZE];
/// writer.write_bytes(0, &write_buffer).unwrap();
/// drop(writer);
/// // Read data by reader
/// let reader = dma_area.read();
/// let mut read_buffer = vec![0u8; 2*PAGE_SIZE];
/// reader.read_bytes(0, &mut read_buffer).unwrap();
/// assert_eq!(read_buffer, write_buffer);
///
/// // Construct a dma_area from VmFrameVec.
/// let vm_frame_vec = VmFrameVec::allocate(&VmAllocOptions::new(24)).unwrap();
/// let dma_area = DmaArea::from_vm_frame_vec(vm_frame_vec);
/// // Write data by writer
/// let writer = dma_area.write();
/// let write_buffer = vec![2u8; 24*PAGE_SIZE];
/// writer.write_bytes(0, &write_buffer).unwrap();
/// drop(writer);
/// // Read data by reader
/// let reader = dma_area.read();
/// let mut read_buffer = vec![0u8; 24*PAGE_SIZE];
/// reader.read_bytes(0, &mut read_buffer).unwrap();
/// assert_eq!(read_buffer, write_buffer);
/// ```
pub struct DmaArea {
    // Using read-write locks to ensure concurrency safety.
    vm_frame_vec: RwLock<VmFrameVec>,
    parent: Weak<DmaAreaPool>,
}

impl DmaArea {
    pub(super) fn new(vm_frame_vec: VmFrameVec, parent: Weak<DmaAreaPool>) -> Self {
        match dma_type() {
            DmaType::Direct => {}
            DmaType::Iommu => {
                // The page table of all devices is the same. So we can use any device ID.
                // FIXME: distinguish different device id.
                let device_id = PciDeviceLocation {
                    bus: 0,
                    device: 0,
                    function: 0,
                };
                let iova_vec = alloc_iova_continuous(device_id, vm_frame_vec.len()).unwrap();
                for (index, frame) in vm_frame_vec.iter().enumerate() {
                    unsafe {
                        if let Err(err) = iommu::map(iova_vec[index], frame.start_paddr()) {
                            match err {
                                iommu::IommuError::NoIommu => {}
                                iommu::IommuError::ModificationError(err) => {
                                    panic!("iommu map error:{:?}", err)
                                }
                            }
                        }
                    }
                }
            }
            DmaType::Tdx => {
                todo!()
            }
        }
        Self {
            vm_frame_vec: RwLock::new(vm_frame_vec),
            parent,
        }
    }

    pub fn from_vm_frame_vec(vm_frame_vec: VmFrameVec) -> Self {
        DmaArea::new(vm_frame_vec, Weak::new())
    }

    pub fn read(&self) -> DmaAreaReader {
        let vm_frame_vec = self.vm_frame_vec.read();
        DmaAreaReader::new(vm_frame_vec)
    }

    pub fn write(&self) -> DmaAreaWriter {
        let vm_frame_vec = self.vm_frame_vec.write();
        DmaAreaWriter::new(vm_frame_vec)
    }
}

impl Drop for DmaArea {
    fn drop(&mut self) {
        if let Some(dma_area_pool) = self.parent.upgrade() {
            dma_area_pool.free(self.vm_frame_vec.write().clone())
        }
    }
}
pub struct DmaAreaReader<'a> {
    vm_frame_vec: RwLockReadGuard<'a, VmFrameVec>,
}

impl<'a> DmaAreaReader<'a> {
    fn new(vm_frame_vec: RwLockReadGuard<'a, VmFrameVec>) -> Self {
        sync_frame_vec(vm_frame_vec.deref());
        Self { vm_frame_vec }
    }
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.vm_frame_vec.read_bytes(offset, buf)?;
        Ok(())
    }
}

pub struct DmaAreaWriter<'a> {
    vm_frame_vec: RwLockWriteGuard<'a, VmFrameVec>,
}

impl<'a> DmaAreaWriter<'a> {
    fn new(vm_frame_vec: RwLockWriteGuard<'a, VmFrameVec>) -> Self {
        Self { vm_frame_vec }
    }
    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.vm_frame_vec.write_bytes(offset, buf)?;
        Ok(())
    }
}

impl<'a> Drop for DmaAreaWriter<'a> {
    fn drop(&mut self) {
        sync_frame_vec(self.vm_frame_vec.deref());
    }
}
