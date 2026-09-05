// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::{device::io_port::WriteOnlyAccess, kernel::ACPI_INFO},
    io::IoPort,
    power::{ExitCode, inject_restart_handler},
};
use spin::Once;

static ACPI_RESET_PORT_AND_VAL: Once<(IoPort<u8, WriteOnlyAccess>, u8)> = Once::new();

fn try_acpi_reset(_code: ExitCode) {
    // If possible, keep this method panic-free because it may be called by the panic handler.
    if let Some((port, val)) = ACPI_RESET_PORT_AND_VAL.get() {
        port.write(*val);
    }
}

pub(super) fn init() {
    let acpi_info = ACPI_INFO.get().unwrap();

    if let Some((reset_port_num, reset_val)) = acpi_info.reset_port_and_val {
        if let Ok(reset_port) = IoPort::acquire(reset_port_num) {
            ACPI_RESET_PORT_AND_VAL.call_once(move || (reset_port, reset_val));
        } else {
            ostd::warn!("The reset port from ACPI is not available");
        }
    }

    // Regular restart handlers run before legacy fallbacks, following Linux's x86 reset order.
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/reboot.c#L657>
    if inject_restart_handler(try_acpi_reset).is_err() {
        ostd::warn!("Failed to register the ACPI restart handler");
    }
}
