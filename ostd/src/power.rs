// SPDX-License-Identifier: MPL-2.0

//! Power management.

use spin::Once;

/// An exit code that denotes the reason for restarting or powering off.
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
/// On success, this function won't return. However, it may return if a restart handler is missing
/// or the handler does not work.
pub fn restart(code: ExitCode) {
    let Some(handler) = RESTART_HANDLER.get() else {
        log::error!("Failed to restart the system because a restart handler is missing");
        return;
    };

    (handler)(code);
    log::error!("Failed to restart the system because the restart handler fails");
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
/// On success, this function won't return. However, it may return if a poweroff handler is missing
/// or the handler does not work.
pub fn poweroff(code: ExitCode) {
    #[cfg(feature = "coverage")]
    crate::coverage::on_system_exit();

    let Some(handler) = POWEROFF_HANDLER.get() else {
        log::error!("Failed to power off the system because a poweroff handler is missing");
        return;
    };

    (handler)(code);
    log::error!("Failed to power off the system because the poweroff handler fails");
}
