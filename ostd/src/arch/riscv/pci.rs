// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use super::boot::DEVICE_TREE;
use crate::prelude::*;

/// Collects all PCI segment group base addresses from the device tree.
///
/// Older variations of PCI were limited to a maximum of 256 PCI bus segments.
/// PCI Express extends this by introducing "PCI Segment Groups", where a system
/// could (in theory) have up to 65536 PCI Segment Groups with 256 PCI bus
/// segments per group. Each PCI segment group can have its own memory-mapped
/// configuration space.
pub(crate) fn collect_segment_group_base_addrs() -> Vec<usize> {
    DEVICE_TREE
        .get()
        .map(|dt| {
            dt.all_nodes()
                .filter(|node| {
                    node.compatible()
                        .is_some_and(|c| c.all().any(|s| s == "pci-host-ecam-generic"))
                })
                .filter_map(|node| node.reg())
                .filter_map(|mut reg| reg.next())
                .map(|r| r.starting_address as usize)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn has_pci_bus() -> bool {
    true
}

// FIXME: This is a QEMU specific address.
pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;
