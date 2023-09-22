mod context_table;
mod fault;
pub(crate) mod iova;
mod remapping;
mod second_stage;

use crate::{sync::Mutex, vm::dma::Daddr};
use log::info;
use spin::Once;

use crate::{
    arch::iommu::{context_table::RootTable, second_stage::PageTableEntry},
    bus::pci::PciDeviceLocation,
    vm::{
        page_table::{PageTableConfig, PageTableError},
        Paddr, PageTable,
    },
};

use self::iova::{dealloc_iova, paddr_to_daddr, remove_paddr, rmap_paddr};

#[derive(Debug)]
pub enum IommuError {
    NoIommu,
    ModificationError(PageTableError),
}

// FIXME: Perform map operations by obtaining ownership of a VmFrame.
///
/// # Safety
///
/// Mapping an incorrect address may lead to a kernel data leak.
pub(crate) unsafe fn map(daddr: Daddr, paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    let device_id = PciDeviceLocation {
        bus: 0,
        device: 0,
        function: 0,
    };
    table.lock().map(device_id, daddr, paddr).map_err(|err| {
        dealloc_iova(device_id, daddr);
        match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        }
    })
}

pub(crate) fn unmap(paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    let device_id = PciDeviceLocation {
        bus: 0,
        device: 0,
        function: 0,
    };
    let daddr = paddr_to_daddr(paddr).unwrap();
    remove_paddr(paddr);
    table.lock().unmap(device_id, daddr).map_err(|err| {
        rmap_paddr(paddr, daddr);
        match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        }
    })
}

pub(crate) fn init() -> Result<(), IommuError> {
    let mut root_table = RootTable::new();
    // For all PCI Device, use the same page table.
    let page_table: PageTable<PageTableEntry> = PageTable::new(PageTableConfig {
        address_width: crate::vm::page_table::AddressWidth::Level3PageTable,
    });
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, page_table.clone())
    }
    remapping::init(&root_table)?;
    iova::init();
    PAGE_TABLE.call_once(|| Mutex::new(root_table));
    info!("IOMMU enabled");
    Ok(())
}

pub(crate) fn has_iommu() -> bool {
    PAGE_TABLE.get().is_some()
}

static PAGE_TABLE: Once<Mutex<RootTable>> = Once::new();
