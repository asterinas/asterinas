// SPDX-License-Identifier: MPL-2.0

// TODO: Add `MappedIrqLine` support for RISC-V.
pub(super) use ostd::trap::irq::IrqLine as MappedIrqLine;

pub(super) fn probe_for_device() {
    use log::debug;
    use ostd::{
        arch::{boot::DEVICE_TREE, plic::PLIC},
        cpu::CpuId,
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
        let region = node.reg().unwrap().next().unwrap();
        let start = region.starting_address as usize;
        let end = start + region.size.unwrap();
        let Ok(io_mem) = IoMem::acquire(start..end) else {
            debug!(
                "[Virtio]: Abort MMIO detection at {:#x} because the MMIO address is not available",
                start
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
                start
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

        let irq_num = node.interrupts().unwrap().next().unwrap();
        let Ok(irq_line) = IrqLine::alloc_specific(irq_num as u8) else {
            debug!(
                "[Virtio]: Ignore MMIO device at {:#x} because its IRQ line is not available",
                start
            );
            return;
        };
        PLIC.get()
            .unwrap()
            .set_interrupt_enabled(CpuId::current_racy().as_usize(), irq_num, true);
        let device = MmioCommonDevice::new(io_mem, irq_line);
        super::MMIO_BUS.lock().register_mmio_device(device);
    });
}
