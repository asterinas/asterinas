// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![cfg(target_arch = "x86_64")]

extern crate alloc;

use component::{ComponentInitError, init_component};
use ostd::{
    arch::device::io_port::WriteOnlyAccess,
    io::IoPort,
    power,
};

use self::controller::{Command, STATUS_OR_COMMAND_PORT_ADDR};

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "i8042: "
    };
}

mod controller;
mod keyboard;
mod mouse;
mod ps2;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    if let Err(err) = controller::init() {
        ostd::warn!("i8042 controller initialization failed: {:?}", err);
    }
    Ok(())
}

/// Attempts to reset the CPU via the i8042 PS/2 controller.
///
/// If the controller was successfully initialized, the reset command is sent through the
/// i8042 driver. If the controller is unavailable or its input buffer is full, the
/// operation falls back to directly writing the CPU reset command (0xFE) to I/O port 0x64
/// without going through the i8042 driver.
pub fn try_cpu_reset(_code: power::ExitCode) {
    if let Some(controller) = controller::I8042_CONTROLLER.get() {
        let mut controller = controller.lock();
        controller.reset_cpu();
        return;
    }

    // Fallback: directly pulse the CPU reset line.
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/reboot.c#L662>
    if let Ok(port) = IoPort::<u8, WriteOnlyAccess>::acquire(STATUS_OR_COMMAND_PORT_ADDR) {
        port.write(Command::CpuReset as u8);
    }
}
