use core::ptr::NonNull;

use crate::{config, vm::paddr_to_vaddr};
use acpi::{AcpiHandler, AcpiTables};
use lazy_static::lazy_static;
use limine::LimineRsdpRequest;
use log::info;
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
    *ACPI_TABLES.lock() =
        unsafe { AcpiTables::from_rsdp(AcpiMemoryHandler {}, rsdp as usize).unwrap() };

    let lock = ACPI_TABLES.lock();
    for (signature, sdt) in lock.sdts.iter() {
        info!("ACPI found signature:{:?}", signature);
    }
    info!("acpi init complete");
}
