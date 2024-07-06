// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support
//!
//! The MP initialization protocol defines two classes of processors:
//! the bootstrap processor (BSP) and the application processors (APs).
//! Following a power-up or RESET of an MP system, system hardware dynamically
//! selects one of the processors on the system bus as the BSP. The remaining
//! processors are designated as APs.
//!
//! The BSP executes the BIOS's boot-strap code to configure the APIC environment,
//! sets up system-wide data structures. Up to now, BSP has completed most of the
//! initialization of the OS, but APs has not been awakened.

use acpi::platform::{PlatformInfo, ProcessorInfo};

use crate::arch::x86::kernel::acpi::ACPI_TABLES;

/// Get processor information
///
/// This function needs to be called after the OS initializes the ACPI table.
pub(crate) fn get_processor_info() -> Option<ProcessorInfo> {
    if !ACPI_TABLES.is_completed() {
        return None;
    }
    Some(
        PlatformInfo::new(&*ACPI_TABLES.get().unwrap().lock())
            .unwrap()
            .processor_info
            .unwrap(),
    )
}
