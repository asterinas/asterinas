use core::ptr::NonNull;

use crate::{config, vm::paddr_to_vaddr};
use acpi::{AcpiHandler, AcpiTables};
use limine::LimineRsdpRequest;
use log::info;
use spin::{Mutex, Once};

/// RSDP information, key is the signature, value is the virtual address of the signature
pub static ACPI_TABLES: Once<Mutex<AcpiTables<AcpiMemoryHandler>>> = Once::new();

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

static RSDP_REQUEST: LimineRsdpRequest = LimineRsdpRequest::new(0);

pub fn init() {
    let response = RSDP_REQUEST
        .get_response()
        .get()
        .expect("Need RSDP address");
    let rsdp = response.address.as_ptr().unwrap().addr() - config::PHYS_OFFSET;
    let acpi_tables =
        unsafe { AcpiTables::from_rsdp(AcpiMemoryHandler {}, rsdp as usize).unwrap() };

    for (signature, sdt) in acpi_tables.sdts.iter() {
        info!("ACPI found signature:{:?}", signature);
    }
    ACPI_TABLES.call_once(|| Mutex::new(acpi_tables));

    info!("acpi init complete");
}
