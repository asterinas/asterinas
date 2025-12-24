// SPDX-License-Identifier: MPL-2.0

//! Power management.

use spin::Once;

use crate::{arch::irq::disable_local_and_halt, cpu::CpuSet};

/// An exit code that denotes the reason for restarting or powering off.
///
/// Whether or not the code is used depends on the hardware. In a virtualization environment, it
/// can be passed to the hypervisor (e.g., as QEMU's exit code). In a bare-metal environment, it
/// can be passed to the firmware. In either case, the code may be silently ignored if reporting
/// the code is not supported.
pub enum ExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failure,
}

static RESTART_HANDLER: Once<fn(ExitCode)> = Once::new();

/// Injects a handler that can restart the system.
///
/// The function may be called only once; subsequent calls take no effect.
///
/// Note that, depending on the specific architecture, OSTD may already have a built-in handler. If
/// so, calling this function outside of OSTD will never take effect. Currently, it happens in
///  - x86_64: Never;
///  - riscv64: Always;
///  - loongarch64: Never.
pub fn inject_restart_handler(handler: fn(ExitCode)) {
    RESTART_HANDLER.call_once(|| handler);
}

/// Restarts the system.
///
/// This function will not return. If a restart handler is missing or not working, it will halt all
/// CPUs on the machine.
pub fn restart(code: ExitCode) -> ! {
    if let Some(handler) = RESTART_HANDLER.get() {
        (handler)(code);
        log::error!("Failed to restart the system because the restart handler fails");
    } else {
        log::error!("Failed to restart the system because a restart handler is missing");
    }

    machine_halt();
}

static POWEROFF_HANDLER: Once<fn(ExitCode)> = Once::new();

/// Injects a handler that can power off the system.
///
/// The function may be called only once; subsequent calls take no effect.
///
/// Note that, depending on the specific architecture, OSTD may already have a built-in handler. If
/// so, calling this function outside of OSTD will never take effect. Currently, it happens in
///  - x86_64: If a QEMU hypervisor is detected;
///  - riscv64: Always;
///  - loongarch64: Never.
pub fn inject_poweroff_handler(handler: fn(ExitCode)) {
    POWEROFF_HANDLER.call_once(|| handler);
}

/// Powers off the system.
///
/// This function will not return. If a poweroff handler is missing or not working, it will halt
/// all CPUs on the machine.
pub fn poweroff(code: ExitCode) -> ! {
    #[cfg(feature = "coverage")]
    crate::coverage::on_system_exit();

    if let Some(handler) = POWEROFF_HANDLER.get() {
        (handler)(code);
        log::error!("Failed to power off the system because the poweroff handler fails");
    } else {
        log::error!("Failed to power off the system because a poweroff handler is missing");
    }

    machine_halt();
}

fn machine_halt() -> ! {
    log::error!("Halting the machine...");

    // TODO: `inter_processor_call` may panic again (e.g., if there is an out-of-memory error). We
    // should find a way to make it panic-free.
    if let Some(ipi_sender) = crate::smp::IPI_SENDER.get() {
        ipi_sender.inter_processor_call(&CpuSet::new_full(), || disable_local_and_halt());
    }
    disable_local_and_halt();
}
