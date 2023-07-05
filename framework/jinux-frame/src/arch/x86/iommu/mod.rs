mod context_table;
mod second_stage;

use log::debug;
use spin::{Mutex, Once};

use crate::{
    arch::{
        iommu::{context_table::RootTable, second_stage::PageTableEntry},
        x86::kernel::acpi::{
            dmar::{Dmar, Remapping},
            ACPI_TABLES,
        },
    },
    bus::pci::PciDeviceLocation,
    vm::{
        paddr_to_vaddr,
        page_table::{PageTableConfig, PageTableError},
        Paddr, PageTable, Vaddr,
    },
};

use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

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
pub(crate) unsafe fn map(vaddr: Vaddr, paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else{
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    table
        .lock()
        .map(
            PciDeviceLocation {
                bus: 0,
                device: 0,
                function: 0,
            },
            vaddr,
            paddr,
        )
        .map_err(|err| match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        })
}

pub(crate) fn unmap(vaddr: Vaddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else{
        return Err(IommuError::NoIommu);
    };
    // The page table of all devices is the same. So we can use any device ID.
    table
        .lock()
        .unmap(
            PciDeviceLocation {
                bus: 0,
                device: 0,
                function: 0,
            },
            vaddr,
        )
        .map_err(|err| match err {
            context_table::ContextTableError::InvalidDeviceId => unreachable!(),
            context_table::ContextTableError::ModificationError(err) => {
                IommuError::ModificationError(err)
            }
        })
}

pub fn init() -> Result<(), IommuError> {
    let mut remapping_reg = RemappingRegisters::new().ok_or(IommuError::NoIommu)?;

    let mut root_table = RootTable::new();

    // For all PCI Device, use the same page table.
    let page_table: PageTable<PageTableEntry> = PageTable::new(PageTableConfig {
        address_width: crate::vm::page_table::AddressWidth::Level3PageTable,
    });
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, page_table.clone())
    }

    let paddr = root_table.paddr();

    // write remapping register
    remapping_reg.root_table_address.write(paddr as u64);
    // start writing
    remapping_reg.global_command.write(0x4000_0000);
    // wait until complete
    while remapping_reg.global_status.read() & 0x4000_0000 == 0 {}

    // enable iommu
    remapping_reg.global_command.write(0x8000_0000);

    debug!("IOMMU registers:{:#x?}", remapping_reg);

    PAGE_TABLE.call_once(|| Mutex::new(root_table));
    Ok(())
}

#[derive(Debug)]
#[repr(C)]
struct RemappingRegisters {
    version: Volatile<&'static u32, ReadOnly>,
    capability: Volatile<&'static u64, ReadOnly>,
    extended_capability: Volatile<&'static u64, ReadOnly>,
    global_command: Volatile<&'static mut u32, WriteOnly>,
    global_status: Volatile<&'static u32, ReadOnly>,
    root_table_address: Volatile<&'static mut u64, ReadWrite>,
    context_command: Volatile<&'static mut u64, ReadWrite>,
}

impl RemappingRegisters {
    /// Create a instance from base address
    fn new() -> Option<Self> {
        let dmar = Dmar::new()?;
        let acpi_table_lock = ACPI_TABLES.get().unwrap().lock();

        debug!("DMAR:{:#x?}", dmar);
        let base_address = {
            let mut addr = 0;
            for remapping in dmar.remapping_iter() {
                match remapping {
                    Remapping::Drhd(drhd) => addr = drhd.register_base_addr(),
                    _ => {}
                }
            }
            if addr == 0 {
                panic!("There should be a DRHD structure in the DMAR table");
            }
            addr
        };

        let vaddr = paddr_to_vaddr(base_address as usize);
        // Safety: All offsets and sizes are strictly adhered to in the manual, and the base address is obtained from Drhd.
        unsafe {
            let version = Volatile::new_read_only(&*(vaddr as *const u32));
            let capability = Volatile::new_read_only(&*((vaddr + 0x08) as *const u64));
            let extended_capability = Volatile::new_read_only(&*((vaddr + 0x10) as *const u64));
            let global_command = Volatile::new_write_only(&mut *((vaddr + 0x18) as *mut u32));
            let global_status = Volatile::new_read_only(&*((vaddr + 0x1C) as *const u32));
            let root_table_address = Volatile::new(&mut *((vaddr + 0x20) as *mut u64));
            let context_command = Volatile::new(&mut *((vaddr + 0x28) as *mut u64));
            Some(Self {
                version,
                capability,
                extended_capability,
                global_command,
                global_status,
                root_table_address,
                context_command,
            })
        }
    }
}

static PAGE_TABLE: Once<Mutex<RootTable>> = Once::new();
