// SPDX-License-Identifier: MPL-2.0

//! Provides i8042 PS/2 Controller I/O port access.
//!
//! Reference: <https://wiki.osdev.org/I8042_PS/2_Controller>
//!

use alloc::string::ToString;

use bitflags::bitflags;
use component::ComponentInitError;
use ostd::{arch::device::io_port::ReadWriteAccess, io::IoPort};
use spin::Once;

/// Data register (R/W)
pub(super) static DATA_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// Status register (R) and Command register (W)
pub(super) static STATUS_OR_COMMAND_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

pub(super) fn init() -> Result<(), ComponentInitError> {
    // TODO: Check the flags in the ACPI table to determine if the PS/2 controller exists. See:
    // <https://uefi.org/specs/ACPI/6.5/05_ACPI_Software_Programming_Model.html#ia-pc-boot-architecture-flags>.

    let data_port = IoPort::acquire(0x60).unwrap();
    let status_or_command_port = IoPort::acquire(0x64).unwrap();

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
        return Err(ComponentInitError::UninitializedDependencies(
            "I8042 controller self-test failed".to_string(),
        ));
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
            return Err(ComponentInitError::UninitializedDependencies(
                "I8042 interface tests failed (port 2)".to_string(),
            ));
        }
        // Enable the second PS/2 port.
        status_or_command_port.write(0xA8);
        config.insert(Configuration::SECOND_PORT_INTERRUPT_ENABLED);
    }

    // Perform interface tests to the first PS/2 port.
    status_or_command_port.write(0xAB);
    if data_port.read() != 0x00 {
        return Err(ComponentInitError::UninitializedDependencies(
            "I8042 interface tests failed (port 1)".to_string(),
        ));
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

    DATA_PORT.call_once(|| data_port);
    STATUS_OR_COMMAND_PORT.call_once(|| status_or_command_port);
    Ok(())
}

bitflags! {
    /// The configuration of the PS/2 controller.
    ///
    /// Commands 0x20 and 0x60 let you read and write the PS/2 Controller Configuration.
    pub(super) struct Configuration: u8 {
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

bitflags! {
    /// The status of the i8042 PS/2 controller.
    ///
    /// Reference: <https://wiki.osdev.org/I8042_PS/2_Controller#Status_Register>.
    pub(super) struct Status: u8 {
        /// Output buffer status (0 = empty, 1 = full)
        /// Must be set before attempting to read data from port 0x60.
        const OUTPUT_BUFFER_IS_FULL = 1 << 0;
        /// System Flag
        /// Meant to be cleared on reset and set by firmware (via. PS/2 Controller Configuration Byte)
        /// if the system passes self tests (POST).
        const SYSTEM_FLAG = 1 << 2;
        /// Time-out error (0 = no error, 1 = time-out error)
        const TIME_OUT_ERROR = 1 << 6;
        /// Parity error (0 = no error, 1 = parity error)
        const PARITY_ERROR = 1 << 7;
    }
}

impl Status {
    pub(super) fn read() -> Self {
        Self::from_bits_truncate(STATUS_OR_COMMAND_PORT.get().unwrap().read())
    }

    pub(super) fn has_data_to_read(&self) -> bool {
        self.contains(Status::OUTPUT_BUFFER_IS_FULL)
    }

    pub(super) fn has_error(&self) -> bool {
        self.contains(Self::SYSTEM_FLAG | Self::TIME_OUT_ERROR | Self::PARITY_ERROR)
    }
}
