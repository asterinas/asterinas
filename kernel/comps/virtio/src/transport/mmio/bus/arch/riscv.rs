// SPDX-License-Identifier: MPL-2.0

pub(super) use ostd::arch::irq::MappedIrqLine;

pub(super) fn probe_for_device() {
    use log::debug;
    use ostd::{
        arch::{
            boot::DEVICE_TREE,
            irq::{InterruptSource, IRQ_CHIP},
        },
        io::IoMem,
        trap::irq::IrqLine,
    };

    use super::common_device::{mmio_check_magic, mmio_read_device_id, MmioCommonDevice};

    let device_tree = DEVICE_TREE.get().unwrap();
    let mmio_nodes = device_tree.all_nodes().filter(|node| {
        node.compatible()
            .and_then(|compatibles| {
                compatibles
                    .all()
                    .find(|compatible| compatible.contains("virtio,mmio"))
            })
            .is_some()
    });
    mmio_nodes.for_each(|node| {
        let interrupt_source = InterruptSource {
            interrupt: node.interrupts().unwrap().next().unwrap() as u32,
            interrupt_parent: node
                .property("interrupt-parent")
                .and_then(|prop| prop.as_usize())
                .unwrap() as u32,
        };
        let mmio_region = node.reg().unwrap().next().unwrap();
        let mmio_start = mmio_region.starting_address as usize;
        let mmio_end = mmio_start + mmio_region.size.unwrap();
        let Ok(io_mem) = IoMem::acquire(mmio_start..mmio_end) else {
            debug!(
                "[Virtio]: Abort MMIO detection at {:#x} because the MMIO address is not available",
                mmio_start
            );
            return;
        };

        // We now check the the rquirements specified in Virtual I/O Device (VIRTIO) Version 1.3,
        // Section 4.2.2.2 Driver Requirements: MMIO Device Register Layout.

        // "The driver MUST ignore a device with MagicValue which is not 0x74726976, although it
        // MAY report an error."
        if !mmio_check_magic(&io_mem) {
            debug!(
                "[Virtio]: Abort MMIO detection at {:#x} because the magic number does not match",
                mmio_start
            );
            return;
        }

        // TODO: "The driver MUST ignore a device with Version which is not 0x2, although it MAY
        // report an error."

        // "The driver MUST ignore a device with DeviceID 0x0, but MUST NOT report any error."
        match mmio_read_device_id(&io_mem) {
            Err(_) | Ok(0) => return,
            Ok(_) => (),
        }

        let Ok(irq_line) = IrqLine::alloc().and_then(|irq_line| {
            IRQ_CHIP
                .get()
                .unwrap()
                .lock()
                .map_interrupt_source_to(interrupt_source, irq_line)
        }) else {
            debug!(
                "[Virtio]: Ignore MMIO device at {:#x} because its IRQ line is not available",
                mmio_start
            );
            return;
        };
        let device = MmioCommonDevice::new(io_mem, irq_line);
        super::MMIO_BUS.lock().register_mmio_device(device);
    });
}
