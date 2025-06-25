// SPDX-License-Identifier: MPL-2.0

//! Provides i8042 PS/2 Controller I/O port access.
//!
//! Reference: <https://wiki.osdev.org/I8042_PS/2_Controller>
//!

use bitflags::bitflags;
use ostd::{
    arch::device::io_port::ReadWriteAccess,
    io::IoPort,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

/// The `I8042Controller` singleton.
pub(super) static I8042_CONTROLLER: Once<SpinLock<I8042Controller, LocalIrqDisabled>> = Once::new();

pub(super) fn init() -> Result<(), I8042ControllerError> {
    let mut controller = I8042Controller::new()?;

    // The steps to initialize the i8042 controller are from:
    // <https://wiki.osdev.org/I8042_PS/2_Controller#Initialising_the_PS/2_Controller>.

    // Disable devices so that they won't send data at the wrong time and mess up initialization.
    controller.wait_and_send_command(Command::DisableFirstPort)?;
    controller.wait_and_send_command(Command::DisableSecondPort)?;

    // Flush the output buffer by reading from the data port and discarding the data.
    controller.flush_output_buffer();

    // Set the controller configuration byte.
    let mut config = controller.read_configuration()?;
    config.remove(
        Configuration::FIRST_PORT_INTERRUPT_ENABLED
            | Configuration::FIRST_PORT_TRANSLATION_ENABLED
            | Configuration::SECOND_PORT_INTERRUPT_ENABLED,
    );
    controller.write_configuration(&config)?;

    // Perform controller self-test.
    controller.wait_and_send_command(Command::TestController)?;
    let result = controller.wait_and_recv_data()?;
    if result != 0x55 {
        // Any value other than 0x55 indicates a self-test fail.
        return Err(I8042ControllerError::ControllerTestFailed);
    }
    // The self-test may reset the controller. Restore the original configuration.
    controller.write_configuration(&config)?;
    // The ports may have been enabled if the controller was reset. Flush the output buffer.
    controller.flush_output_buffer();

    // Determine if there are two channels.
    controller.wait_and_send_command(Command::EnableSecondPort)?;
    let has_second_port = config.contains(Configuration::SECOND_PORT_CLOCK_DISABLED)
        && !controller
            .read_configuration()?
            .contains(Configuration::SECOND_PORT_CLOCK_DISABLED);
    controller.wait_and_send_command(Command::DisableSecondPort)?;
    // Flush the output buffer again since we may have enabled the second port.
    controller.flush_output_buffer();

    // Perform interface tests to the first PS/2 port.
    controller.wait_and_send_command(Command::TestFirstPort)?;
    let result = controller.wait_and_recv_data()?;
    if result != 0x00 {
        return Err(I8042ControllerError::FirstPortTestFailed);
    }

    // Perform interface tests to the second PS/2 port (if it exists).
    if has_second_port {
        controller.wait_and_send_command(Command::TestSecondPort)?;
        let result = controller.wait_and_recv_data()?;
        if result != 0x00 {
            return Err(I8042ControllerError::SecondPortTestFailed);
        }
    }

    // Enable the first PS/2 port.
    controller.wait_and_send_command(Command::EnableFirstPort)?;
    if let Err(err) = super::keyboard::init(&mut controller) {
        log::warn!("i8042 keyboard initialization failed: {:?}", err);
        controller.wait_and_send_command(Command::DisableFirstPort)?;
    } else {
        config.remove(Configuration::FIRST_PORT_CLOCK_DISABLED);
        config.insert(
            Configuration::FIRST_PORT_INTERRUPT_ENABLED
                | Configuration::FIRST_PORT_TRANSLATION_ENABLED,
        );
    }

    // TODO: Add a mouse driver and enable the second PS/2 port (if it exists).

    I8042_CONTROLLER.call_once(|| SpinLock::new(controller));
    let mut controller = I8042_CONTROLLER.get().unwrap().lock();
    // Write the new configuration to enable the interrupts after setting up `I8042_CONTROLLER`.
    controller.write_configuration(&config)?;
    // Flush the output buffer to ensure that new data can trigger interrupts.
    controller.flush_output_buffer();

    Ok(())
}

/// An I8042 PS/2 Controller.
pub(super) struct I8042Controller {
    data_port: IoPort<u8, ReadWriteAccess>,
    status_or_command_port: IoPort<u8, ReadWriteAccess>,
}

/// The maximum number of times to wait for the i8042 controller to be ready.
const MAX_WAITING_COUNT: usize = 64;

impl I8042Controller {
    fn new() -> Result<Self, I8042ControllerError> {
        // TODO: Check the flags in the ACPI table to determine if the PS/2 controller exists. See:
        // <https://uefi.org/specs/ACPI/6.5/05_ACPI_Software_Programming_Model.html#ia-pc-boot-architecture-flags>.
        let controller = Self {
            data_port: IoPort::acquire(0x60).unwrap(),
            status_or_command_port: IoPort::acquire(0x64).unwrap(),
        };
        Ok(controller)
    }

    fn read_configuration(&mut self) -> Result<Configuration, I8042ControllerError> {
        self.wait_and_send_command(Command::ReadConfiguration)?;
        self.wait_and_recv_data()
            .map(Configuration::from_bits_retain)
    }

    fn write_configuration(&mut self, config: &Configuration) -> Result<(), I8042ControllerError> {
        self.wait_and_send_command(Command::WriteConfiguration)?;
        self.wait_and_send_data(config.bits())
    }

    fn wait_and_send_command(&mut self, command: Command) -> Result<(), I8042ControllerError> {
        for _ in 0..MAX_WAITING_COUNT {
            if self.send_command(command).is_ok() {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(I8042ControllerError::OutputBusy)
    }

    fn send_command(&mut self, command: Command) -> Result<(), I8042ControllerError> {
        if !self.read_status().contains(Status::INPUT_BUFFER_IS_FULL) {
            self.write_command(command as u8);
            Ok(())
        } else {
            Err(I8042ControllerError::OutputBusy)
        }
    }

    fn read_status(&self) -> Status {
        Status::from_bits_retain(self.status_or_command_port.read())
    }

    fn write_command(&mut self, command: u8) {
        self.status_or_command_port.write(command);
    }

    pub(super) fn wait_and_send_data(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        for _ in 0..MAX_WAITING_COUNT {
            if self.send_data(data).is_ok() {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(I8042ControllerError::OutputBusy)
    }

    pub(super) fn send_data(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        if !self.read_status().contains(Status::INPUT_BUFFER_IS_FULL) {
            self.write_data(data);
            Ok(())
        } else {
            Err(I8042ControllerError::OutputBusy)
        }
    }

    fn write_data(&mut self, data: u8) {
        self.data_port.write(data);
    }

    pub(super) fn wait_and_recv_data(&mut self) -> Result<u8, I8042ControllerError> {
        for _ in 0..MAX_WAITING_COUNT {
            if let Some(data) = self.receive_data() {
                return Ok(data);
            }
            core::hint::spin_loop();
        }
        Err(I8042ControllerError::NoInput)
    }

    pub(super) fn receive_data(&mut self) -> Option<u8> {
        if self.read_status().contains(Status::OUTPUT_BUFFER_IS_FULL) {
            Some(self.read_data())
        } else {
            None
        }
    }

    fn read_data(&self) -> u8 {
        self.data_port.read()
    }

    fn flush_output_buffer(&mut self) {
        while self.receive_data().is_some() {}
    }
}

/// Errors that can occur when initializing the i8042 controller.
#[derive(Debug, Clone, Copy)]
pub(super) enum I8042ControllerError {
    ControllerTestFailed,
    FirstPortTestFailed,
    SecondPortTestFailed,
    OutputBusy,
    NoInput,
    DeviceResetFailed,
    DeviceUnknown,
    DeviceAllocIrqFailed,
}

/// The commands that can be sent to the PS/2 controller.
///
/// Reference: <https://wiki.osdev.org/I8042_PS/2_Controller#PS/2_Controller_Commands>.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum Command {
    ReadConfiguration = 0x20,
    WriteConfiguration = 0x60,
    DisableSecondPort = 0xA7,
    EnableSecondPort = 0xA8,
    TestSecondPort = 0xA9,
    TestController = 0xAA,
    TestFirstPort = 0xAB,
    DisableFirstPort = 0xAD,
    EnableFirstPort = 0xAE,
}

bitflags! {
    /// The configuration of the PS/2 controller.
    ///
    /// Reference: <https://wiki.osdev.org/I8042_PS/2_Controller#PS/2_Controller_Configuration_Byte>.
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

bitflags! {
    /// The status of the i8042 PS/2 controller.
    ///
    /// Reference: <https://wiki.osdev.org/I8042_PS/2_Controller#Status_Register>.
    struct Status: u8 {
        /// Output buffer status (0 = empty, 1 = full)
        /// Must be set before attempting to read data from port 0x60.
        const OUTPUT_BUFFER_IS_FULL = 1 << 0;
        /// Input buffer status (0 = empty, 1 = full)
        /// Must be clear before attempting to write data to IO port 0x60 or IO port 0x64.
        const INPUT_BUFFER_IS_FULL = 1 << 1;
        /// System Flag
        /// Meant to be cleared on reset and set by firmware (via. PS/2 Controller Configuration Byte)
        /// if the system passes self tests (POST).
        const SYSTEM_FLAG = 1 << 2;
        /// Command or data (0 = data, 1 = command)
        /// Data written to input buffer is data for PS/2 device or command for controller.
        const IS_COMMAND = 1 << 3;
        /// Time-out error (0 = no error, 1 = time-out error)
        const TIME_OUT_ERROR = 1 << 6;
        /// Parity error (0 = no error, 1 = parity error)
        const PARITY_ERROR = 1 << 7;
    }
}
