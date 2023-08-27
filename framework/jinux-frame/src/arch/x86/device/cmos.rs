use acpi::{fadt::Fadt, sdt::Signature};
use x86_64::instructions::port::{ReadOnlyAccess, WriteOnlyAccess};

use crate::arch::x86::kernel::acpi::ACPI_TABLES;

use super::io_port::IoPort;

pub static CMOS_ADDRESS: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x70) };
pub static CMOS_DATA: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x71) };

pub fn get_century() -> u8 {
    const DEFAULT_21_CENTURY: u8 = 50;
    if !ACPI_TABLES.is_completed() {
        return DEFAULT_21_CENTURY;
    }
    unsafe {
        match ACPI_TABLES
            .get()
            .unwrap()
            .lock()
            .get_sdt::<Fadt>(Signature::FADT)
        {
            Ok(a) => {
                let century = a.unwrap().century;
                if century == 0 {
                    DEFAULT_21_CENTURY
                } else {
                    century
                }
            }
            Err(er) => DEFAULT_21_CENTURY,
        }
    }
}
