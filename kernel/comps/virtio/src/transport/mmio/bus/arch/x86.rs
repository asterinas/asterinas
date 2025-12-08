// SPDX-License-Identifier: MPL-2.0

use log::debug;
use ostd::arch::irq::IRQ_CHIP;
pub(super) use ostd::arch::irq::MappedIrqLine;

use crate::transport::mmio::bus::MmioRegisterError;

pub(super) fn probe_for_device() {
    // TODO: The correct method for detecting VirtIO-MMIO devices on x86_64 systems is to parse the
    // kernel command line if ACPI tables are absent [1], or the ACPI SSDT if ACPI tables are
    // present [2]. Neither of them is supported for now. This function's approach of blindly
    // scanning the MMIO region is only a workaround.
    // [1]: https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/drivers/virtio/virtio_mmio.c#L733
    // [2]: https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/drivers/virtio/virtio_mmio.c#L840

    // Constants from QEMU MicroVM. We should remove them as they're QEMU's implementation details.
    //
    // https://github.com/qemu/qemu/blob/3c5a5e213e5f08fbfe70728237f7799ac70f5b99/hw/i386/microvm.c#L201
    const QEMU_MMIO_BASE: usize = 0xFEB0_0000;
    const QEMU_MMIO_SIZE: usize = 512;
    // https://github.com/qemu/qemu/blob/3c5a5e213e5f08fbfe70728237f7799ac70f5b99/hw/i386/microvm.c#L196
    const QEMU_IOAPIC1_GSI_BASE: u32 = 16;
    const QEMU_IOAPIC1_NUM_TRANS: u32 = 8;
    // https://github.com/qemu/qemu/blob/3c5a5e213e5f08fbfe70728237f7799ac70f5b99/hw/i386/microvm.c#L192
    const QEMU_IOAPIC2_GSI_BASE: u32 = 24;
    const QEMU_IOAPIC2_NUM_TRANS: u32 = 24;

    let irq_chip = IRQ_CHIP.get().unwrap();
    let (gsi_base, num_trans) = match irq_chip.count_io_apics() {
        1 => (QEMU_IOAPIC1_GSI_BASE, QEMU_IOAPIC1_NUM_TRANS),
        2.. => (QEMU_IOAPIC2_GSI_BASE, QEMU_IOAPIC2_NUM_TRANS),
        0 => {
            debug!("[Virtio]: Skip MMIO detection because there are no I/O APICs");
            return;
        }
    };

    for index in 0..num_trans {
        let mmio_base = QEMU_MMIO_BASE + (index as usize) * QEMU_MMIO_SIZE;
        match super::try_register_mmio_device(mmio_base..(mmio_base + QEMU_MMIO_SIZE), |irq_line| {
            irq_chip.map_gsi_pin_to(irq_line, gsi_base + index)
        }) {
            Err(e) if e.is_fatal() => break,
            _ => continue,
        }
    }
}

impl MmioRegisterError {
    /// Returns `true` if it should terminate a linear MMIO scan.
    fn is_fatal(self) -> bool {
        matches!(self, Self::MmioUnavailable | Self::MagicMismatch)
    }
}
