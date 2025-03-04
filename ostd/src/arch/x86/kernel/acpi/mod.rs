// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

pub mod dmar;
pub mod remapping;

use core::ptr::NonNull;

use acpi::{platform::PlatformInfo, rsdp::Rsdp, AcpiHandler, AcpiTables};
use log::{info, warn};
use spin::Once;

use crate::{
    boot::{self, BootloaderAcpiArg},
    mm::paddr_to_vaddr,
};

/// RSDP information, key is the signature, value is the virtual address of the signature
pub static PLATFORM_INFO: Once<PlatformInfo<'static, alloc::alloc::Global>> = Once::new();

#[derive(Debug, Clone)]
pub struct AcpiMemoryHandler {}

impl AcpiHandler for AcpiMemoryHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        acpi::PhysicalMapping::new(
            physical_address,
            NonNull::new(paddr_to_vaddr(physical_address) as *mut T).unwrap(),
            size,
            size,
            self.clone(),
        )
    }

    fn unmap_physical_region<T>(region: &acpi::PhysicalMapping<Self, T>) {}
}

pub(crate) fn get_acpi_tables() -> Option<AcpiTables<AcpiMemoryHandler>> {
    let acpi_tables = match boot::EARLY_INFO.get().unwrap().acpi_arg {
        BootloaderAcpiArg::Rsdp(addr) => unsafe {
            AcpiTables::from_rsdp(AcpiMemoryHandler {}, addr).unwrap()
        },
        BootloaderAcpiArg::Rsdt(addr) => unsafe {
            AcpiTables::from_rsdt(AcpiMemoryHandler {}, 0, addr).unwrap()
        },
        BootloaderAcpiArg::Xsdt(addr) => unsafe {
            AcpiTables::from_rsdt(AcpiMemoryHandler {}, 1, addr).unwrap()
        },
        BootloaderAcpiArg::NotProvided => {
            // We search by ourselves if the bootloader decides not to provide a rsdp location.
            let rsdp = unsafe { Rsdp::search_for_on_bios(AcpiMemoryHandler {}) };
            match rsdp {
                Ok(map) => unsafe {
                    AcpiTables::from_rsdp(AcpiMemoryHandler {}, map.physical_start()).unwrap()
                },
                Err(_) => {
                    warn!("ACPI info not found!");
                    return None;
                }
            }
        }
    };

    Some(acpi_tables)
}

/// Initializes the platform information by parsing ACPI tables in to the heap.
///
/// Must be called after the heap is initialized.
pub(crate) fn init() {
    let Some(acpi_tables) = get_acpi_tables() else {
        return;
    };

    for header in acpi_tables.headers() {
        info!("ACPI found signature:{:?}", header.signature);
    }

    let platform_info = PlatformInfo::new(&acpi_tables).unwrap();
    PLATFORM_INFO.call_once(|| platform_info);

    info!("ACPI initialization complete");
}

/// Gets the platform information.
///
/// Must be called after [`init()`]. Otherwise, there may not be any platform
/// information even if the system has ACPI tables.
pub(crate) fn get_platform_info() -> Option<&'static PlatformInfo<'static, alloc::alloc::Global>> {
    PLATFORM_INFO.get()
}
