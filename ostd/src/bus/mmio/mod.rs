// SPDX-License-Identifier: MPL-2.0

//! Virtio over MMIO

use bus::MmioBus;

use crate::sync::SpinLock;

pub mod bus;
pub mod common_device;

/// The MMIO bus instance.
pub static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());

pub(crate) fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        // TODO: support virtio-mmio devices on TDX.
        //
        // Currently, virtio-mmio devices need to acquire sub-page MMIO regions,
        // which are not supported by `IoMem::acquire` in the TDX environment.
    } else {
        x86_probe();
    });
}

#[cfg(target_arch = "x86_64")]
fn x86_probe() {
    use common_device::{mmio_check_magic, mmio_read_device_id, MmioCommonDevice};
    use log::debug;

    use crate::{io::IoMem, trap::IrqLine};

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
    const QEMU_IOAPIC1_IRQ_BASE: u8 = 16;
    const QEMU_IOAPIC1_NUM_TRANS: u8 = 8;
    // https://github.com/qemu/qemu/blob/3c5a5e213e5f08fbfe70728237f7799ac70f5b99/hw/i386/microvm.c#L192
    const QEMU_IOAPIC2_IRQ_BASE: u8 = 0;
    const QEMU_IOAPIC2_NUM_TRANS: u8 = 24;

    let mut mmio_bus = MMIO_BUS.lock();

    let io_apics = crate::arch::kernel::IO_APIC.get();
    let (mut ioapic, irq_base, num_trans) = match io_apics {
        Some(io_apic_vec) if io_apic_vec.len() == 1 => (
            io_apic_vec[0].lock(),
            QEMU_IOAPIC1_IRQ_BASE,
            QEMU_IOAPIC1_NUM_TRANS,
        ),
        Some(io_apic_vec) if io_apic_vec.len() >= 2 => (
            io_apic_vec[1].lock(),
            QEMU_IOAPIC2_IRQ_BASE,
            QEMU_IOAPIC2_NUM_TRANS,
        ),
        Some(_) | None => {
            debug!("[Virtio]: Skip MMIO detection because there are no I/O APICs");
            return;
        }
    };

    for index in 0..num_trans {
        let mmio_base = QEMU_MMIO_BASE + (index as usize) * QEMU_MMIO_SIZE;
        let Ok(io_mem) = IoMem::acquire(mmio_base..(mmio_base + QEMU_MMIO_SIZE)) else {
            debug!(
                "[Virtio]: Abort MMIO detection at {:#x} because the MMIO address is not available",
                mmio_base
            );
            break;
        };

        // We now check the the rquirements specified in Virtual I/O Device (VIRTIO) Version 1.3,
        // Section 4.2.2.2 Driver Requirements: MMIO Device Register Layout.

        // "The driver MUST ignore a device with MagicValue which is not 0x74726976, although it
        // MAY report an error."
        if !mmio_check_magic(&io_mem) {
            debug!(
                "[Virtio]: Abort MMIO detection at {:#x} because the magic number does not match",
                mmio_base
            );
            break;
        }

        // TODO: "The driver MUST ignore a device with Version which is not 0x2, although it MAY
        // report an error."

        // "The driver MUST ignore a device with DeviceID 0x0, but MUST NOT report any error."
        match mmio_read_device_id(&io_mem) {
            Err(_) | Ok(0) => continue,
            Ok(_) => (),
        }

        let irq_line = if let Ok(irq_line) = IrqLine::alloc()
            && ioapic.enable(irq_base + index, irq_line.clone()).is_ok()
        {
            irq_line
        } else {
            debug!(
                "[Virtio]: Ignore MMIO device at {:#x} because its IRQ line is not available",
                mmio_base
            );
            continue;
        };

        let device = MmioCommonDevice::new(io_mem, irq_line);
        mmio_bus.register_mmio_device(device);
    }
}
