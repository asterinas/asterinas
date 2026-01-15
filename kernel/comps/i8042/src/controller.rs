// SPDX-License-Identifier: MPL-2.0

//! Provides i8042 PS/2 Controller I/O port access.
//!
//! Reference: <https://wiki.osdev.org/I8042_PS/2_Controller>
//!

use aster_cmdline::{KCMDLINE, ModuleArg};
use bitflags::bitflags;
use ostd::{
    arch::{device::io_port::ReadWriteAccess, kernel::ACPI_INFO},
    io::IoPort,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

/// The `I8042Controller` singleton.
pub(super) static I8042_CONTROLLER: Once<SpinLock<I8042Controller, LocalIrqDisabled>> = Once::new();

pub(super) fn init() -> Result<(), I8042ControllerError> {
    const SELF_TEST_OK: u8 = 0x55;
    const PORT_TEST_OK: u8 = 0x00;

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
    if result != SELF_TEST_OK {
        // Any value other than `SELF_TEST_OK` indicates a self-test fail.
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
    if result != PORT_TEST_OK {
        return Err(I8042ControllerError::FirstPortTestFailed);
    }

    // Perform interface tests to the second PS/2 port (if it exists).
    if has_second_port {
        controller.wait_and_send_command(Command::TestSecondPort)?;
        let result = controller.wait_and_recv_data()?;
        if result != PORT_TEST_OK {
            return Err(I8042ControllerError::SecondPortTestFailed);
        }
    }

    // Enable the first PS/2 port (keyboard).
    controller.wait_and_send_command(Command::EnableFirstPort)?;
    if let Err(err) = super::keyboard::init(&mut controller) {
        log::warn!("i8042 keyboard initialization failed: {:?}", err);
    } else {
        config.remove(Configuration::FIRST_PORT_CLOCK_DISABLED);
        config.insert(
            Configuration::FIRST_PORT_INTERRUPT_ENABLED
                | Configuration::FIRST_PORT_TRANSLATION_ENABLED,
        );
    }
    // Temporarily disable the first PS/2 port to avoid interference.
    controller.wait_and_send_command(Command::DisableFirstPort)?;
    controller.flush_output_buffer();

    // Enable the second PS/2 port (mouse) if it exists.
    if has_second_port {
        controller.wait_and_send_command(Command::EnableSecondPort)?;
        if let Err(err) = super::mouse::init(&mut controller) {
            log::warn!("i8042 mouse initialization failed: {:?}", err);
        } else {
            config.remove(Configuration::SECOND_PORT_CLOCK_DISABLED);
            config.insert(Configuration::SECOND_PORT_INTERRUPT_ENABLED);
        }
        // Temporarily disable the second PS/2 port to avoid interference.
        controller.wait_and_send_command(Command::DisableFirstPort)?;
        controller.flush_output_buffer();
    }

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

impl I8042Controller {
    fn new() -> Result<Self, I8042ControllerError> {
        const DATA_PORT_ADDR: u16 = 0x60;
        const STATUS_OR_COMMAND_PORT_ADDR: u16 = 0x64;

        if !Self::is_present_acpi() {
            // The PS/2 controller does not exist. See:
            // <https://uefi.org/specs/ACPI/6.5/05_ACPI_Software_Programming_Model.html#ia-pc-boot-architecture-flags>.
            //
            // However, it may actually be present and enumerable from other sources, such as PnP
            // devices. See:
            // <https://elixir.bootlin.com/linux/v6.18/source/drivers/input/serio/i8042-acpipnpio.h#L1578>.
            //
            // Currently, we lack the necessary support, so we allow the user to manually override
            // the ACPI flag by appending "i8042.exist" to the kernel command line.
            //
            // TODO: Add support for enumerating PnP devices and remove the command line option.
            if !Self::is_present_cmdline() {
                log::info!(
                    "ACPI says i8042 controller is absent; \
                    if it is incorrect, append 'i8042.exist' in cmdline to override it"
                );
                return Err(I8042ControllerError::NotPresent);
            } else {
                log::info!(
                    "ACPI says i8042 controller is absent; \
                    however, it is overridden by 'i8042.exist' in cmdline"
                );
            }
        } else {
            log::info!("ACPI says i8042 controller is present");
        }

        let controller = Self {
            data_port: IoPort::acquire(DATA_PORT_ADDR).unwrap(),
            status_or_command_port: IoPort::acquire(STATUS_OR_COMMAND_PORT_ADDR).unwrap(),
        };
        Ok(controller)
    }

    fn is_present_acpi() -> bool {
        ACPI_INFO
            .get()
            .unwrap()
            .boot_flags
            .is_some_and(|flags| !flags.motherboard_implements_8042())
    }

    /// Checks if the kernel command line contains the "i8042.exist" option.
    fn is_present_cmdline() -> bool {
        KCMDLINE
            .get()
            .unwrap()
            .get_module_args("i8042")
            .is_some_and(|args| {
                args.iter()
                    .any(|arg| matches!(arg, ModuleArg::Arg(s) if s.as_bytes() == b"exist"))
            })
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
        spin_wait_until(Timeout::Short, || self.send_command(command).ok())
            .ok_or(I8042ControllerError::OutputBusy)
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
        spin_wait_until(Timeout::Short, || self.send_data(data).ok())
            .ok_or(I8042ControllerError::OutputBusy)
    }

    pub(super) fn write_to_second_port(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        self.wait_and_send_command(Command::WriteToSecondPort)?;
        self.wait_and_send_data(data)
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
        spin_wait_until(Timeout::Short, || self.receive_data()).ok_or(I8042ControllerError::NoInput)
    }

    /// Waits a long time for the data to be received.
    ///
    /// This is usually used when performing a reset that takes a long time to complete.
    pub(super) fn wait_long_and_recv_data(&mut self) -> Result<u8, I8042ControllerError> {
        spin_wait_until(Timeout::Long, || self.receive_data()).ok_or(I8042ControllerError::NoInput)
    }

    /// Waits for the specified data to be received.
    ///
    /// This is typically used when waiting for an acknowledgment of a command. Any garbage data
    /// before the acknowledgment is ignored.
    pub(super) fn wait_for_specific_data(
        &mut self,
        data: &[u8],
    ) -> Result<u8, I8042ControllerError> {
        spin_wait_until(Timeout::Short, || {
            self.receive_data().filter(|val| data.contains(val))
        })
        .ok_or(I8042ControllerError::NoInput)
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

/// Timeout in milliseconds for sending commands or receiving data.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17.9/source/drivers/input/serio/libps2.c#L344>
#[repr(u16)]
#[derive(Debug, Clone, Copy)]
enum Timeout {
    /// Short timeout for normal commands (500 ms).
    Short = 500,
    /// Long timeout for the reset command (4000 ms).
    Long = 4000,
}

/// Spins and waits until the timeout occurs or `f` returns `Some(_)`.
//
// TODO: The timeout is relatively large, up to several seconds. Therefore, spinning here is not
// appropriate. The code needs to be refactored to use asynchronous interrupts.
fn spin_wait_until<F, R>(timeout: Timeout, mut f: F) -> Option<R>
where
    F: FnMut() -> Option<R>,
{
    use ostd::arch::{read_tsc, tsc_freq};

    const MSEC_PER_SEC: u64 = 1000;

    if let Some(res) = f() {
        return Some(res);
    }

    let current = read_tsc();
    let distance = tsc_freq() / MSEC_PER_SEC * (timeout as u16 as u64);
    loop {
        if let Some(res) = f() {
            return Some(res);
        }
        if read_tsc().wrapping_sub(current) >= distance {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// Errors that can occur when initializing the i8042 controller.
#[derive(Debug, Clone, Copy)]
pub(super) enum I8042ControllerError {
    NotPresent,
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
    WriteToSecondPort = 0xD4,
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
