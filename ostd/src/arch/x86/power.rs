// SPDX-License-Identifier: MPL-2.0

//! Power management.

mod qemu_isa_debug {
    //! The isa-debug-exit device in QEMU.
    //!
    //! Reference: <https://elixir.bootlin.com/qemu/v10.1.2/source/hw/misc/debugexit.c>

    use spin::Once;

    use crate::{arch::device::io_port::WriteOnlyAccess, io::IoPort, power::ExitCode};

    // For `qemu-system-x86_64`, the exit code will be `(code << 1) | 1`. So it is not possible to
    // let QEMU invoke `exit(0)`. We also need to check if the exit code is returned by the kernel,
    // so we cannot use `0` as `EXIT_SUCCESS` because it may conflict with QEMU's return value `1`,
    // which indicates that QEMU itself fails.
    const EXIT_SUCCESS: u32 = 0x10;
    const EXIT_FAILURE: u32 = 0x20;

    static DEBUG_EXIT_PORT: Once<IoPort<u32, WriteOnlyAccess>> = Once::new();

    pub(super) fn try_exit_qemu(code: ExitCode) {
        let value = match code {
            ExitCode::Success => EXIT_SUCCESS,
            ExitCode::Failure => EXIT_FAILURE,
        };

        // If possible, keep this method panic-free because it may be called by the panic handler.
        if let Some(port) = DEBUG_EXIT_PORT.get() {
            port.write(value);
        }
    }

    pub(super) fn init() {
        const DEBUG_EXIT_PORT_NUM: u16 = 0xF4;

        let debug_exit_port = IoPort::acquire(DEBUG_EXIT_PORT_NUM).unwrap();

        DEBUG_EXIT_PORT.call_once(|| debug_exit_port);
    }
}

pub(super) fn init() {
    use super::{cpu::cpuid, kernel::ACPI_INFO};

    if !cpuid::query_if_running_in_qemu() {
        return;
    }
    // The "KVMKVMKVM" signature is shared by every KVM-based VMM, including
    // EC2 Nitro, where there is no isa-debug-exit device and installing this
    // handler would shadow the real (ACPI) power-off path. QEMU's firmware
    // tables carry the OEM ID "BOCHS "; require it when ACPI is available.
    if let Some(info) = ACPI_INFO.get()
        && let Some(oem_id) = info.oem_id
        && &oem_id != b"BOCHS "
    {
        crate::info!(
            "KVM hypervisor with firmware OEM {:?}: not QEMU, no isa-debug-exit",
            core::str::from_utf8(&oem_id).unwrap_or("?")
        );
        return;
    }

    // FIXME: We assume that the kernel is running in QEMU with the following QEMU command line
    // arguments that specify the isa-debug-exit device:
    // `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
    crate::info!("QEMU hypervisor detected, assuming that the isa-debug-exit device exists");

    qemu_isa_debug::init();
}

/// Attempts to power off the system using an architecture-specific mechanism.
///
/// On x86, this function attempts to power off the system through QEMU's ISA debug-exit device if
/// QEMU was detected during initialization. Otherwise, it does nothing and returns.
pub fn try_poweroff(code: crate::power::ExitCode) {
    qemu_isa_debug::try_exit_qemu(code);
}

/// Attempts to restart the system using an architecture-specific mechanism.
///
/// On x86, this triple-faults the current CPU: it loads an empty IDT and raises a software
/// interrupt, which the CPU cannot deliver, nor the resulting double fault, so it resets. This
/// is the last-resort reboot method on x86 (Linux does the same) and the only one that works
/// where neither an ACPI reset register nor an i8042 controller exists, such as on EC2 Nitro.
///
/// This method does not return. Nothing runs after it on the current CPU, and the other CPUs
/// stop when the chipset resets.
pub fn try_restart(_code: crate::power::ExitCode) {
    let null_idt: [u8; 10] = [0; 10];

    // SAFETY: We are intentionally bringing the machine down. The empty IDT guarantees that the
    // `int3` below cannot be handled and the CPU resets instead.
    unsafe {
        core::arch::asm!(
            "cli",
            "lidt [{}]",
            "int3",
            in(reg) null_idt.as_ptr(),
            options(nostack, noreturn)
        );
    }
}
