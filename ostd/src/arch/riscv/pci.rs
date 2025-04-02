// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use super::boot::DEVICE_TREE;
use crate::prelude::*;

pub(crate) fn segment_group_base_addr_vec() -> Vec<usize> {
    let mut addr_vec = Vec::new();
    let device_tree = DEVICE_TREE.get().unwrap();
    for node in device_tree.all_nodes().filter(|node| {
        node.compatible()
            .is_some_and(|c| c.all().any(|s| s == "pci-host-ecam-generic"))
    }) {
        let addr = node.reg().unwrap().next().unwrap().starting_address as usize;
        println!("PCI host bridge found at {:#x}", addr);
        addr_vec.push(addr);
    }
    addr_vec
}

pub(crate) fn has_pci_bus() -> bool {
    true
}

// FIXME: This is a QEMU specific address.
pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;
