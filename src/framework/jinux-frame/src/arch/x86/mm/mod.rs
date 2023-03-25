use alloc::vec::Vec;
use limine::{LimineMemmapEntry, LimineMemmapRequest, LimineMemoryMapEntryType};
use log::debug;
use spin::Once;
use x86_64::structures::paging::PhysFrame;

use crate::vm::{MemoryRegions, MemoryRegionsType, Paddr};

pub unsafe fn activate_page_table(root_paddr: Paddr, flags: x86_64::registers::control::Cr3Flags) {
    x86_64::registers::control::Cr3::write(
        PhysFrame::from_start_address(x86_64::PhysAddr::new(root_paddr as u64)).unwrap(),
        flags,
    );
}

static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest::new(0);
static MEMORY_REGIONS: Once<Vec<MemoryRegions>> = Once::new();

impl From<&LimineMemmapEntry> for MemoryRegions {
    fn from(value: &LimineMemmapEntry) -> Self {
        Self {
            base: value.base,
            len: value.len,
            typ: MemoryRegionsType::from(value.typ),
        }
    }
}

impl From<LimineMemoryMapEntryType> for MemoryRegionsType {
    fn from(value: LimineMemoryMapEntryType) -> Self {
        match value {
            LimineMemoryMapEntryType::Usable => Self::Usable,
            LimineMemoryMapEntryType::Reserved => Self::Reserved,
            LimineMemoryMapEntryType::AcpiReclaimable => Self::AcpiReclaimable,
            LimineMemoryMapEntryType::AcpiNvs => Self::AcpiNvs,
            LimineMemoryMapEntryType::BadMemory => Self::BadMemory,
            LimineMemoryMapEntryType::BootloaderReclaimable => Self::BootloaderReclaimable,
            LimineMemoryMapEntryType::KernelAndModules => Self::KernelAndModules,
            LimineMemoryMapEntryType::Framebuffer => Self::Framebuffer,
        }
    }
}

/// Get memory regions, this function should call after the heap was initialized
pub fn get_memory_regions() -> &'static Vec<MemoryRegions> {
    let mut memory_regions = Vec::new();
    let response = MEMMAP_REQUEST
        .get_response()
        .get()
        .expect("Not found memory region information");
    for i in response.memmap() {
        debug!("Found memory region:{:x?}", **i);
        memory_regions.push(MemoryRegions::from(&**i));
    }
    MEMORY_REGIONS.call_once(|| memory_regions);
    MEMORY_REGIONS.get().unwrap()
}
