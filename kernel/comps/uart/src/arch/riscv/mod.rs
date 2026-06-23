// SPDX-License-Identifier: MPL-2.0

use ostd::arch::boot::DEVICE_TREE;

mod ns16550a;
mod sifive;

pub(super) fn init() {
    let device_tree = DEVICE_TREE.get().unwrap();

    if let Some(ns16550a_node) = device_tree.find_compatible(&ns16550a::FDT_COMPATIBLES) {
        ns16550a::init(ns16550a_node);
        return;
    }

    if let Some(sifive_node) = device_tree.find_compatible(&sifive::FDT_COMPATIBLES) {
        sifive::init(sifive_node);
    }
}
