// SPDX-License-Identifier: MPL-2.0

//! Provides CMOS information.
//!
//! "CMOS" is a tiny bit of very low power static memory that lives on the same chip as the
//! Real-Time Clock (RTC).
//!
//! Reference: <https://wiki.osdev.org/CMOS>
//!

use acpi::fadt::Fadt;

use crate::arch::kernel::acpi::get_acpi_tables;

/// Gets the century register location.
///
/// This function is used to get the century value from the Real-Time Clock (RTC).
pub fn century_register() -> Option<u8> {
    let acpi_tables = get_acpi_tables()?;
    match acpi_tables.find_table::<Fadt>() {
        Ok(fadt) => Some(fadt.century),
        Err(_) => None,
    }
}
