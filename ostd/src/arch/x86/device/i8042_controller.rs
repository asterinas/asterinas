// SPDX-License-Identifier: MPL-2.0

//! Provides I8042 PS/2 Controller I/O port access.
//!
//! Reference: <https://wiki.osdev.org/I8042_PS/2_Controller>
//!

use acpi::fadt::Fadt;
use bitflags::bitflags;
use spin::Once;

use super::io_port::ReadWriteAccess;
use crate::{
    arch::kernel::{acpi::get_acpi_tables, pic, IO_APIC},
    io::IoPort,
    sync::{LocalIrqDisabled, SpinLock},
    trap::IrqLine,
};

/// Data register (R/W)
pub static DATA_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// Status register (R) and Command register (W)
pub static STATUS_OR_COMMAND_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// IRQ line for i8042 keyboard.
pub static KEYBOARD_IRQ_LINE: Once<SpinLock<IrqLine, LocalIrqDisabled>> = Once::new();

pub(crate) fn init() {
    let Some(acpi_tables) = get_acpi_tables() else {
        return;
    };

    let Ok(fadt) = acpi_tables.find_table::<Fadt>() else {
        return;
    };

    // Reference to packed field is unaligned, copy the field contents to a local variable.
    let iapc_boot_arch = fadt.iapc_boot_arch;
    // Determine if the PS/2 Controller Exists. If set, indicates that the motherboard
    // contains support for a port 0x60 and 0x64 based keyboard controller.
    if !iapc_boot_arch.motherboard_implements_8042() {
        return;
    }

    // SAFETY: The I/O ports are exist and valid.
    let (data_port, status_or_command_port) = unsafe { (IoPort::new(0x60), IoPort::new(0x64)) };

    // Disable devices so that they won't send data at the wrong time and mess up initialisation.
    status_or_command_port.write(0xAD);
    status_or_command_port.write(0xA7);

    // Flush the the output buffer by reading from port 0x60 and discarding the data.
    let _ = data_port.read();

    // Set the controller configuration byte.
    status_or_command_port.write(0x20);
    let mut config = Configuration::from_bits_truncate(data_port.read());
    config.remove(
        Configuration::FIRST_PORT_INTERRUPT_ENABLED
            | Configuration::FIRST_PORT_TRANSLATION_ENABLED
            | Configuration::SECOND_PORT_INTERRUPT_ENABLED,
    );
    status_or_command_port.write(0x60);
    data_port.write(config.bits());

    // Perform controller self-test.
    // Any value other than 0x55 indicates a self-test fail. This can reset the PS/2 controller
    // on some hardware (tested on a 2016 laptop). At the very least, the Controller Configuration
    // Byte should be restored for compatibility with such hardware.
    status_or_command_port.write(0xAA);
    if data_port.read() != 0x55 {
        return;
    }

    // Determine if there are 2 channels.
    status_or_command_port.write(0xA8);
    status_or_command_port.write(0x20);
    let mut config = Configuration::from_bits_truncate(data_port.read());
    if !config.contains(Configuration::SECOND_PORT_CLOCK_DISABLED) {
        status_or_command_port.write(0xA7);
        config.remove(
            Configuration::SECOND_PORT_INTERRUPT_ENABLED
                | Configuration::SECOND_PORT_CLOCK_DISABLED,
        );
        status_or_command_port.write(0x60);
        data_port.write(config.bits());

        // Perform interface tests to the second PS/2 port.
        status_or_command_port.write(0xA9);
        if data_port.read() != 0x00 {
            return;
        }
        // Enable the second PS/2 port.
        status_or_command_port.write(0xA8);
        config.insert(Configuration::SECOND_PORT_INTERRUPT_ENABLED);
    }

    // Perform interface tests to the first PS/2 port.
    status_or_command_port.write(0xAB);
    if data_port.read() != 0x00 {
        return;
    }
    // Enable the first PS/2 port.
    status_or_command_port.write(0xAE);
    config.remove(Configuration::FIRST_PORT_CLOCK_DISABLED);
    config.insert(
        Configuration::FIRST_PORT_INTERRUPT_ENABLED | Configuration::FIRST_PORT_TRANSLATION_ENABLED,
    );
    status_or_command_port.write(0x60);
    data_port.write(config.bits());

    // The controller initialisation is done, all PS/2 devices (if any) should be reset by the driver.

    let irq_line = if !IO_APIC.is_completed() {
        pic::allocate_irq(1).unwrap()
    } else {
        let irq_line = IrqLine::alloc().unwrap();
        let mut io_apic = IO_APIC.get().unwrap()[0].lock();
        io_apic.enable(1, irq_line.clone()).unwrap();
        irq_line
    };

    DATA_PORT.call_once(|| data_port);
    STATUS_OR_COMMAND_PORT.call_once(|| status_or_command_port);
    KEYBOARD_IRQ_LINE.call_once(|| SpinLock::new(irq_line));
}

bitflags! {
    /// The configuration of the PS/2 controller.
    ///
    /// Commands 0x20 and 0x60 let you read and write the PS/2 Controller Configuration.
    struct Configuration: u8 {
        /// First PS/2 port interrupt (1 = enabled, 0 = disabled)
        const FIRST_PORT_INTERRUPT_ENABLED = 1 << 0;
        /// Second PS/2 port interrupt (1 = enabled, 0 = disabled, only if 2 PS/2 ports supported)
        const SECOND_PORT_INTERRUPT_ENABLED = 1 << 1;
        /// System Flag (1 = system passed POST, 0 = your OS shouldn't be running)
        const SYSTEM_POST_PASSED = 1 << 2;
        /// First PS/2 port clock (1 = disabled, 0 = enabled)
        const FIRST_PORT_CLOCK_DISABLED = 1 << 4;
        /// Second PS/2 port clock (1 = disabled, 0 = enabled, only if 2 PS/2 ports supported)
        const SECOND_PORT_CLOCK_DISABLED = 1 << 5;
        /// First PS/2 port translation (1 = enabled, 0 = disabled)
        const FIRST_PORT_TRANSLATION_ENABLED = 1 << 6;
    }
}
