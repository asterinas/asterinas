// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use crate::{arch::kernel::acpi::get_acpi_tables, prelude::*};

pub(crate) fn segment_group_base_addr_vec() -> Vec<usize> {
    let acpi_tables = get_acpi_tables().unwrap();
    acpi_tables
        .find_table::<acpi::mcfg::Mcfg>()
        .unwrap()
        .get()
        .entries()
        .iter()
        .map(|entry| entry.base_address as usize)
        .collect()
}

pub(crate) fn has_pci_bus() -> bool {
    true
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0xFEE0_0000;
