// SPDX-License-Identifier: MPL-2.0

use ostd::arch::boot::DEVICE_TREE;

mod dw_apb_uart;
mod ns16550a;
pub(super) fn init() {
    let device_tree = DEVICE_TREE.get().unwrap();

    if let Some(node) = device_tree.find_compatible(&["snps,dw-apb-uart"]) {
        dw_apb_uart::init(node);
        return;
    }

    if let Some(node) = device_tree.find_compatible(&["ns16550a"]) {
        ns16550a::init(node);
    }
}
