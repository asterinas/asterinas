// SPDX-License-Identifier: MPL-2.0

pub mod dmar;
pub mod remapping;

use core::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use crate::vm::paddr_to_vaddr;
use crate::{
    boot::{self, BootloaderAcpiArg},
    sync::SpinLock,
};
use acpi::{sdt::SdtHeader, AcpiHandler, AcpiTable, AcpiTables};
use alloc::borrow::ToOwned;
use log::{info, warn};
use spin::Once;

/// RSDP information, key is the signature, value is the virtual address of the signature
pub static ACPI_TABLES: Once<SpinLock<AcpiTables<AcpiMemoryHandler>>> = Once::new();

/// Sdt header wrapper, user can use this structure to easily derive Debug, get table information without creating a new struture.
///
/// For example, in DMAR (DMA Remapping) structure,
/// we can use the following code to get some information of DMAR, including address, length:
///
/// ```rust
/// acpi_table.get_sdt::<SdtHeaderWrapper>(Signature::DMAR).unwrap()
/// ```
///
#[derive(Clone, Copy)]
pub struct SdtHeaderWrapper(SdtHeader);

impl AcpiTable for SdtHeaderWrapper {
    fn header(&self) -> &acpi::sdt::SdtHeader {
        &self.0
    }
}

impl Deref for SdtHeaderWrapper {
    type Target = SdtHeader;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SdtHeaderWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl core::fmt::Debug for SdtHeaderWrapper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let length = self.0.length;
        let oem_revision = self.0.oem_revision;
        let creator_id = self.0.creator_id;
        let creator_revision = self.0.creator_revision;

        f.debug_struct("Dmar")
            .field("signature", &self.0.signature)
            .field("length", &length)
            .field("revision", &self.0.revision)
            .field("checksum", &self.0.checksum)
            .field("oem_id", &self.0.oem_id())
            .field("oem_table_id", &self.0.oem_table_id())
            .field("oem_revision", &oem_revision)
            .field("creator_id", &creator_id)
            .field("creator_revision", &creator_revision)
            .finish()
    }
}

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
            let rsdp = unsafe { rsdp::Rsdp::search_for_on_bios(AcpiMemoryHandler {}) };
            match rsdp {
                Ok(map) => match map.validate() {
                    Ok(_) => {
                        if map.revision() > 0 {
                            unsafe {
                                AcpiTables::from_rsdt(
                                    AcpiMemoryHandler {},
                                    1,
                                    map.xsdt_address() as usize,
                                )
                                .unwrap()
                            }
                        } else {
                            unsafe {
                                AcpiTables::from_rsdt(
                                    AcpiMemoryHandler {},
                                    0,
                                    map.rsdt_address() as usize,
                                )
                                .unwrap()
                            }
                        }
                    }
                    Err(_) => {
                        warn!("ACPI info not found!");
                        return;
                    }
                },
                Err(_) => {
                    warn!("ACPI info not found!");
                    return;
                }
            }
        }
    };

    for (signature, sdt) in acpi_tables.sdts.iter() {
        info!("ACPI found signature:{:?}", signature);
    }
    ACPI_TABLES.call_once(|| SpinLock::new(acpi_tables));

    info!("acpi init complete");
}
