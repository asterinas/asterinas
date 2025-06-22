// SPDX-License-Identifier: MPL-2.0

//! Provides CMOS I/O port access.
//!
//! "CMOS" is a tiny bit of very low power static memory that lives on the same chip as the Real-Time Clock (RTC).
//!
//! Reference: <https://wiki.osdev.org/CMOS>
//!

#![expect(unused_variables)]

use acpi::fadt::Fadt;
use x86_64::instructions::port::{ReadOnlyAccess, WriteOnlyAccess};

use crate::{
    arch::kernel::acpi::get_acpi_tables,
    io::{sensitive_io_port, IoPort},
};

sensitive_io_port!(unsafe {
    /// CMOS address I/O port
    pub static CMOS_ADDRESS: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x70);
    /// CMOS data I/O port
    pub static CMOS_DATA: IoPort<u8, ReadOnlyAccess> = IoPort::new(0x71);
});

/// Gets the century register location. This function is used in RTC(Real Time Clock) module initialization.
pub fn century_register() -> Option<u8> {
    let acpi_tables = get_acpi_tables()?;
    match acpi_tables.find_table::<Fadt>() {
        Ok(a) => Some(a.century),
        Err(er) => None,
    }
}
