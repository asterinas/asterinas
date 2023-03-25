use acpi::{fadt::Fadt, sdt::Signature};
use x86_64::instructions::port::{ReadOnlyAccess, WriteOnlyAccess};

use crate::driver::ACPI_TABLES;

use super::io_port::IoPort;

pub static CMOS_ADDRESS: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x70) };
pub static CMOS_DATA: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x71) };

pub fn get_century() -> u8 {
    unsafe {
        let a = ACPI_TABLES
            .get()
            .unwrap()
            .lock()
            .get_sdt::<Fadt>(Signature::FADT)
            .unwrap()
            .expect("not found FACP in ACPI table");
        a.century
    }
}
