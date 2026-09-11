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
#[derive(Clone, Copy)]
pub enum ExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failure,
}

static RESTART_HANDLER: Once<fn(ExitCode)> = Once::new();

/// Injects a handler that can restart the system.
///
/// By default, [`restart`] uses [`crate::arch::power::try_restart`]. An injected handler replaces
/// this default and may call it as part of its own policy.
///
/// The function may be called only once; subsequent calls take no effect.
pub fn inject_restart_handler(handler: fn(ExitCode)) {
    RESTART_HANDLER.call_once(|| handler);
}

/// Restarts the system.
///
/// This function will not return. If the selected restart mechanism returns, it will halt all CPUs
/// on the machine.
pub fn restart(code: ExitCode) -> ! {
    if let Some(handler) = RESTART_HANDLER.get() {
        (handler)(code);
    } else {
        crate::arch::power::try_restart(code);
    }
    crate::error!("Failed to restart the system because the restart mechanism fails");

    machine_halt();
}

static POWEROFF_HANDLER: Once<fn(ExitCode)> = Once::new();

/// Injects a handler that can power off the system.
///
/// By default, [`poweroff`] uses [`crate::arch::power::try_poweroff`]. An injected handler replaces
/// this default and may call it as part of its own policy.
///
/// The function may be called only once; subsequent calls take no effect.
pub fn inject_poweroff_handler(handler: fn(ExitCode)) {
    POWEROFF_HANDLER.call_once(|| handler);
}

/// Powers off the system.
///
/// This function will not return. If the selected poweroff mechanism returns, it will halt all CPUs
/// on the machine.
pub fn poweroff(code: ExitCode) -> ! {
    #[cfg(feature = "coverage")]
    crate::coverage::on_system_exit();

    if let Some(handler) = POWEROFF_HANDLER.get() {
        (handler)(code);
    } else {
        crate::arch::power::try_poweroff(code);
    }
    crate::error!("Failed to power off the system because the poweroff mechanism fails");

    machine_halt();
}

fn machine_halt() -> ! {
    crate::error!("Halting the machine...");

    // TODO: `inter_processor_call` may panic again (e.g., if there is an out-of-memory error). We
    // should find a way to make it panic-free.
    if let Some(ipi_sender) = crate::smp::IPI_SENDER.get() {
        ipi_sender.inter_processor_call(&CpuSet::new_full(), || disable_local_and_halt());
    }
    disable_local_and_halt();
}
