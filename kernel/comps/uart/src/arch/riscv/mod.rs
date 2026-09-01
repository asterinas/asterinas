// SPDX-License-Identifier: MPL-2.0

use ostd::arch::boot::DEVICE_TREE;

mod ns16550a;

pub(super) fn init() {
    let device_tree = DEVICE_TREE.get().unwrap();

    if let Some(ns16550a_node) = device_tree.find_compatible(&["ns16550a"]) {
        ns16550a::init(ns16550a_node);
    }
}
