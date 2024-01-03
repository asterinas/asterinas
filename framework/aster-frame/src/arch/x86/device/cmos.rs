// SPDX-License-Identifier: MPL-2.0

use acpi::{fadt::Fadt, sdt::Signature};
use x86_64::instructions::port::{ReadOnlyAccess, WriteOnlyAccess};

use crate::arch::x86::kernel::acpi::ACPI_TABLES;

use super::io_port::IoPort;

pub static CMOS_ADDRESS: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x70) };
pub static CMOS_DATA: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x71) };

pub fn get_century_register() -> Option<u8> {
    if !ACPI_TABLES.is_completed() {
        return None;
    }
    unsafe {
        match ACPI_TABLES
            .get()
            .unwrap()
            .lock()
            .get_sdt::<Fadt>(Signature::FADT)
        {
            Ok(a) => Some(a.unwrap().century),
            Err(er) => None,
        }
    }
}
