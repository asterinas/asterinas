// SPDX-License-Identifier: MPL-2.0

pub(super) use ostd::arch::irq::MappedIrqLine;
use ostd::arch::{
    boot::DEVICE_TREE,
    irq::{IRQ_CHIP, InterruptSourceInFdt, parse_gic_intid_from_fdt},
};

pub(super) fn probe_for_device() {
    // Parse device tree for virtio,mmio nodes.
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
        // Parse MMIO region.
        let mmio_region = node.reg().unwrap().next().unwrap();
        let mmio_start = mmio_region.starting_address as usize;
        let mmio_end = mmio_start + mmio_region.size.unwrap();

        // Parse interrupt from FDT.
        // GIC uses 3-cell format: <type number flags>.
        // node.interrupts() returns None for 3-cell format, so parse raw bytes.
        let int_prop = node
            .property("interrupts")
            .expect("virtio-mmio has no interrupts property");
        let intid = parse_gic_intid_from_fdt(int_prop.value)
            .expect("virtio-mmio has invalid interrupt property");

        // Get interrupt parent phandle.
        // On ARM64 QEMU virt, the GIC is at /intc with a phandle property.
        // The interrupt-parent may be inherited from ancestors or explicitly set.
        // Walk up the tree or fallback to /intc node directly.
        let interrupt_parent: usize = node
            .property("interrupt-parent")
            .and_then(|p| p.as_usize())
            .unwrap_or_else(|| {
                // Fallback: use the /intc node's phandle directly.
                device_tree
                    .find_node("/intc")
                    .and_then(|n| n.property("phandle"))
                    .and_then(|p| p.as_usize())
                    .expect("virtio-mmio has no interrupt parent")
            });

        let interrupt_source_in_fdt = InterruptSourceInFdt {
            interrupt_parent: interrupt_parent as u32,
            interrupt: intid,
        };

        let _ = super::try_register_mmio_device(mmio_start..mmio_end, |irq_line| {
            IRQ_CHIP
                .get()
                .unwrap()
                .map_fdt_pin_to(interrupt_source_in_fdt, irq_line)
        });
    });
}
