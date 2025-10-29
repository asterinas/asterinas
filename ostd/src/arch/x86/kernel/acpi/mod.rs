// SPDX-License-Identifier: MPL-2.0

pub(in crate::arch) mod dmar;
pub(in crate::arch) mod remapping;

use core::{num::NonZeroU8, ptr::NonNull};

use acpi::{
    fadt::{Fadt, IaPcBootArchFlags},
    rsdp::Rsdp,
    AcpiHandler, AcpiTables,
};
use log::warn;
use spin::Once;

use crate::{
    boot::{self, BootloaderAcpiArg},
    mm::paddr_to_vaddr,
};

#[derive(Debug, Clone)]
pub(crate) struct AcpiMemoryHandler {}

impl AcpiHandler for AcpiMemoryHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        let virtual_address = NonNull::new(paddr_to_vaddr(physical_address) as *mut T).unwrap();

        // SAFETY: The caller should guarantee that `physical_address..physical_address + size` is
        // part of the ACPI table. Then the memory region is mapped to `virtual_address` and is
        // valid for read and immutable dereferencing.
        // FIXME: The caller guarantee only holds if we trust the hardware to provide a valid ACPI
        // table. Otherwise, if the table is corrupted, it may reference arbitrary memory regions.
        unsafe {
            acpi::PhysicalMapping::new(physical_address, virtual_address, size, size, self.clone())
        }
    }

    fn unmap_physical_region<T>(_region: &acpi::PhysicalMapping<Self, T>) {}
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

/// The platform information provided by the ACPI tables.
///
/// Currently, this structure contains only a limited set of fields, far fewer than those in all
/// ACPI tables. However, the goal is to expand it properly to keep the simplicity of the OSTD code
/// while enabling OSTD users to safely retrieve information from the ACPI tables.
#[derive(Debug)]
pub struct AcpiInfo {
    /// The RTC CMOS RAM index to the century of data value; the "CENTURY" field in the FADT.
    pub century_register: Option<NonZeroU8>,
    /// IA-PC Boot Architecture Flags; the "IAPC_BOOT_ARCH" field in the FADT.
    pub boot_flags: Option<IaPcBootArchFlags>,
}

/// The [`AcpiInfo`] singleton.
pub static ACPI_INFO: Once<AcpiInfo> = Once::new();

pub(in crate::arch) fn init() {
    let mut acpi_info = AcpiInfo {
        century_register: None,
        boot_flags: None,
    };

    if let Some(acpi_tables) = get_acpi_tables()
        && let Ok(fadt) = acpi_tables.find_table::<Fadt>()
    {
        // A zero means that the century register does not exist.
        acpi_info.century_register = NonZeroU8::new(fadt.century);
        acpi_info.boot_flags = Some(fadt.iapc_boot_arch);
    };

    log::info!("[ACPI]: Collected information {:?}", acpi_info);

    ACPI_INFO.call_once(|| acpi_info);
}
