// SPDX-License-Identifier: MPL-2.0

//! Architecture kernel module.
//
// TODO: The purpose of this module is too ambiguous. We should split it up and move its submodules
// to more suitable locations.

pub(super) mod acpi;
pub(super) mod apic;
pub(super) mod tsc;

pub use acpi::{AcpiInfo, ACPI_INFO};
