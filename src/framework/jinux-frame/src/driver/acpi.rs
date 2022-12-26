use core::ptr::NonNull;

use crate::{info, mm::address::phys_to_virt};
use acpi::{AcpiHandler, AcpiTables};
use lazy_static::lazy_static;
use spin::Mutex;

lazy_static! {
    /// RSDP information, key is the signature, value is the virtual address of the signature
    pub(crate) static ref ACPI_TABLES : Mutex<AcpiTables<AcpiMemoryHandler>> = unsafe{
        Mutex::new(core::mem::MaybeUninit::zeroed().assume_init())
    };
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
            NonNull::new(phys_to_virt(physical_address) as *mut T).unwrap(),
            size,
            size,
            self.clone(),
        )
    }

    fn unmap_physical_region<T>(region: &acpi::PhysicalMapping<Self, T>) {}
}

pub fn init(rsdp: u64) {
    let a = unsafe { AcpiTables::from_rsdp(AcpiMemoryHandler {}, rsdp as usize).unwrap() };
    *ACPI_TABLES.lock() = a;

    let c = ACPI_TABLES.lock();
    for (signature, sdt) in c.sdts.iter() {
        info!("ACPI found signature:{:?}", signature);
    }
    info!("acpi init complete");
}
