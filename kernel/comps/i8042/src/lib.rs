// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![cfg(target_arch = "x86_64")]

extern crate alloc;

use component::{ComponentInitError, init_component};
use ostd::{
    arch::{
        device::io_port::{ReadWriteAccess, WriteOnlyAccess},
        irq::{IRQ_CHIP, MappedIrqLine},
        trap::TrapFrame,
    },
    io::IoPort,
    irq::IrqLine,
    power,
};
use spin::Once;

use self::controller::{
    Command, Configuration, DATA_PORT_ADDR, I8042_CONTROLLER, STATUS_OR_COMMAND_PORT_ADDR,
};

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

/// IRQ line for the i8042 keyboard.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// ISA interrupt number for i8042 keyboard.
const ISA_INTR_NUM: u8 = 1;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    // Register the keyboard IRQ handler independently of controller initialization.
    // This ensures IRQ1 is always handled, even when the i8042 controller
    // does not fully implement the PS/2 device protocol.
    if let Ok(mut irq_line) = IrqLine::alloc().and_then(|irq_line| {
        IRQ_CHIP
            .get()
            .unwrap()
            .map_isa_pin_to(irq_line, ISA_INTR_NUM)
    }) {
        irq_line.on_active(handle_keyboard_interrupt);
        IRQ_LINE.call_once(|| irq_line);
        ostd::info!("Keyboard IRQ1 handler registered");
    } else {
        ostd::warn!("Failed to register keyboard IRQ1");
    }

    // Attempt full controller initialization (keyboard driver, mouse, etc.).
    // The controller may have modified the i8042 control register and disabled
    // the keyboard interrupt during init. If the init fails we need to restore
    // the interrupt bits so that IRQ1-based keyboard handling can still work.
    if let Err(err) = controller::init() {
        ostd::warn!("i8042 controller initialization failed: {:?}", err);
        restore_keyboard_interrupt();
    }
    Ok(())
}

/// Restores i8042 control byte to ensure the keyboard can generate interrupts
/// after a failed controller initialization.
fn restore_keyboard_interrupt() {
    if let Ok(port) = IoPort::<u8, ReadWriteAccess>::acquire(STATUS_OR_COMMAND_PORT_ADDR) {
        port.write(Command::WriteConfiguration as u8);
    }
    if let Ok(port) = IoPort::<u8, ReadWriteAccess>::acquire(DATA_PORT_ADDR) {
        let config = Configuration::FIRST_PORT_INTERRUPT_ENABLED
            | Configuration::SYSTEM_POST_PASSED
            | Configuration::FIRST_PORT_TRANSLATION_ENABLED;
        port.write(config.bits());
    }
}

/// The IRQ1 handler for the i8042 keyboard.
fn handle_keyboard_interrupt(_trap_frame: &TrapFrame) {
    keyboard::dispatch_keyboard_input();
}

/// Attempts to reset the CPU via the i8042 PS/2 controller.
///
/// If the controller was successfully initialized, the reset command is sent through the
/// i8042 driver. If the controller is unavailable or its input buffer is full, the
/// operation falls back to directly writing the CPU reset command (0xFE) to I/O port 0x64
/// without going through the i8042 driver.
pub fn try_cpu_reset(_code: power::ExitCode) {
    if let Some(controller) = I8042_CONTROLLER.get() {
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
