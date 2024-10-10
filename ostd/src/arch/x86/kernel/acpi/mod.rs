// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

pub mod dmar;
pub mod remapping;

use alloc::borrow::ToOwned;
use core::ptr::NonNull;

use acpi::{rsdp::Rsdp, AcpiHandler, AcpiTables};
use log::{info, warn};
use spin::Once;

use crate::{
    boot::{self, BootloaderAcpiArg},
    mm::paddr_to_vaddr,
    sync::SpinLock,
};

/// RSDP information, key is the signature, value is the virtual address of the signature
pub static ACPI_TABLES: Once<SpinLock<AcpiTables<AcpiMemoryHandler>>> = Once::new();

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

pub fn init() {
    let acpi_tables = match boot::acpi_arg().to_owned() {
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
                    return;
                }
            }
        }
    };

    for header in acpi_tables.headers() {
        info!("ACPI found signature:{:?}", header.signature);
    }
    ACPI_TABLES.call_once(|| SpinLock::new(acpi_tables));

    info!("acpi init complete");
}
