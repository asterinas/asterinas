// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use crate::{arch::kernel::acpi::get_acpi_tables, prelude::*};

/// Collects all PCI segment group base addresses from the ACPI MCFG table.
///
/// Older variations of PCI were limited to a maximum of 256 PCI bus segments.
/// PCI Express extends this by introducing "PCI Segment Groups", where a system
/// could (in theory) have up to 65536 PCI Segment Groups with 256 PCI bus
/// segments per group. Each PCI segment group can have its own memory-mapped
/// configuration space.
pub(crate) fn collect_segment_group_base_addrs() -> Vec<usize> {
    get_acpi_tables()
        .map(|tables| {
            tables
                .find_table::<acpi::mcfg::Mcfg>()
                .map(|mcfg| {
                    mcfg.get()
                        .entries()
                        .iter()
                        .map(|entry| entry.base_address as usize)
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

pub(crate) fn has_pci_bus() -> bool {
    true
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0xFEE0_0000;
