// SPDX-License-Identifier: MPL-2.0

use fdt_util::{AcquireIoMems, AcquireIrqLines};
use ostd::arch::boot::DEVICE_TREE;
pub(super) use ostd::arch::irq::MappedIrqLine;

use crate::transport::mmio::{
    MMIO_BUS, bus::common_device::MmioCommonDevice, layout::VirtioMmioLayout,
};

pub(super) fn probe_for_device() {
    // The device tree parsing logic here assumed a Linux-compatible device
    // tree.
    // Reference: <https://www.kernel.org/doc/Documentation/devicetree/bindings/virtio/mmio.txt>.
    let device_tree = DEVICE_TREE.get().unwrap();
    let mmio_nodes = device_tree.all_nodes().filter(|node| {
        node.compatible().is_some_and(|compatibles| {
            compatibles
                .all()
                .any(|compatible| compatible == "virtio,mmio")
        })
    });
    mmio_nodes.for_each(|node| {
        let Some([io_mem]) = node.acquire_io_mems([size_of::<VirtioMmioLayout>()]) else {
            return;
        };
        if super::validate_mmio_device(&io_mem).is_err() {
            return;
        }

        let Some([irq_line]) = node.acquire_irq_lines() else {
            return;
        };

        let device = MmioCommonDevice::new(io_mem, irq_line);
        MMIO_BUS.lock().register_mmio_device(device);
    });
}
