// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::{device::io_port::WriteOnlyAccess, kernel::ACPI_INFO},
    io::IoPort,
    power::{inject_restart_handler, ExitCode},
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
    let Some((reset_port_num, reset_val)) = acpi_info.reset_port_and_val else {
        return;
    };
    let Ok(reset_port) = IoPort::acquire(reset_port_num) else {
        log::warn!("The reset port from ACPI is not available");
        return;
    };

    ACPI_RESET_PORT_AND_VAL.call_once(move || (reset_port, reset_val));
    inject_restart_handler(try_acpi_reset);
}
