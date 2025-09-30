// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, RelCode, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{
    arch::{
        kernel::{MappedIrqLine, IRQ_CHIP},
        trap::TrapFrame,
    },
    irq::IrqLine,
};
use spin::Once;

use super::controller::{I8042Controller, I8042ControllerError, I8042_CONTROLLER};
use crate::alloc::string::ToString;

const PACKET_LEN_STANDARD: usize = 3;
const PACKET_LEN_INTELLIMOUSE: usize = 4;
const PACKET_LEN_MAX: usize = 4;

const DEVICE_ID_STANDARD: u8 = 0x00;
const DEVICE_ID_INTELLIMOUSE: u8 = 0x03;
const DEVICE_ID_INTELLIMOUSE_EXPLORER: u8 = 0x04;

const PACKET_SYNC_MASK: u8 = 0x08;
const BUTTON_LEFT_MASK: u8 = 0x01;
const BUTTON_RIGHT_MASK: u8 = 0x02;
const BUTTON_MIDDLE_MASK: u8 = 0x04;

const INDEX_STATUS: usize = 0;
const INDEX_X: usize = 1;
const INDEX_Y: usize = 2;
const INDEX_WHEEL: usize = 3;

/// IRQ line for i8042 mouse.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 mouse (second PS/2 port).
const ISA_INTR_NUM: u8 = 12;

/// Mouse packet state machine.
static MOUSE_PACKET_STATE: spin::Mutex<MousePacketState> =
    spin::Mutex::new(MousePacketState::new());

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Reset mouse device by sending 0xFF (reset command) to the second PS/2 port.
    send_mouse_command(controller, 0xFF)?;

    // The response should be 0xFA (ACK) and 0xAA (BAT successful), followed by the device PS/2 ID.
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // The reset command may take some time to finish. Try again a few times.
    if (0..5).find_map(|_| controller.wait_and_recv_data().ok()) != Some(0xAA) {
        return Err(I8042ControllerError::DeviceResetFailed);
    }

    let mut device_id = controller
        .wait_and_recv_data()
        .unwrap_or(DEVICE_ID_STANDARD);
    log::info!("PS/2 mouse device ID: 0x{:02X}", device_id);

    // Try to enable IntelliMouse by setting sample rates: 200, 100, 80.
    // Reference: https://wiki.osdev.org/Mouse_Input
    if enable_intellimouse(controller).is_ok() {
        // Query device ID again.
        if let Ok(new_id) = get_mouse_device_id(controller) {
            device_id = new_id;
            log::info!("PS/2 mouse upgraded device ID: 0x{:02X}", device_id);
        }
    }

    // Enable data reporting.
    send_mouse_command(controller, 0xF4)?;
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }

    let mut irq_line = IrqLine::alloc()
        .and_then(|irq_line| {
            IRQ_CHIP
                .get()
                .unwrap()
                .map_isa_pin_to(irq_line, ISA_INTR_NUM)
        })
        .map_err(|_| I8042ControllerError::DeviceAllocIrqFailed)?;
    irq_line.on_active(handle_mouse_input);
    IRQ_LINE.call_once(|| irq_line);

    // Create and register the i8042 mouse device.
    let mouse_device = Arc::new(I8042Mouse::new());
    let registered_device = aster_input::register_device(mouse_device);
    REGISTERED_DEVICE.call_once(|| registered_device);

    // Configure packet length based on device ID.
    if device_id == DEVICE_ID_INTELLIMOUSE || device_id == DEVICE_ID_INTELLIMOUSE_EXPLORER {
        let mut state = MOUSE_PACKET_STATE.lock();
        state.set_packet_len(PACKET_LEN_INTELLIMOUSE);
    }

    Ok(())
}

/// Sends a command to the mouse (second PS/2 port).
fn send_mouse_command(
    controller: &mut I8042Controller,
    command: u8,
) -> Result<(), I8042ControllerError> {
    controller.write_to_second_port(command)?;
    Ok(())
}

/// Sends a command with one argument to the mouse.
fn send_mouse_command_with_arg(
    controller: &mut I8042Controller,
    command: u8,
    arg: u8,
) -> Result<(), I8042ControllerError> {
    // Send the command.
    controller.write_to_second_port(command)?;
    // Expect ACK (0xFA).
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // Send the argument.
    controller.write_to_second_port(arg)?;
    // Expect ACK (0xFA).
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    Ok(())
}

/// Gets the mouse device ID.
fn get_mouse_device_id(controller: &mut I8042Controller) -> Result<u8, I8042ControllerError> {
    // Send Get Device ID (0xF2). Device replies ACK (0xFA) then ID byte.
    controller.write_to_second_port(0xF2)?;
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    controller.wait_and_recv_data()
}

/// Enables IntelliMouse mode (wheel) by sending the magic sample rate sequence.
fn enable_intellimouse(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Set Sample Rate (0xF3) with 200, then 100, then 80.
    send_mouse_command_with_arg(controller, 0xF3, 200)?;
    send_mouse_command_with_arg(controller, 0xF3, 100)?;
    send_mouse_command_with_arg(controller, 0xF3, 80)?;
    Ok(())
}

#[derive(Debug)]
struct I8042Mouse {
    name: String,
    phys: String,
    uniq: String,
    id: InputId,
    capability: InputCapability,
}

impl I8042Mouse {
    fn new() -> Self {
        let mut capability = InputCapability::new();

        // Mouse supports key events and relative movement events.
        capability.set_supported_event_type(aster_input::event_type_codes::EventTypes::KEY);
        capability.set_supported_event_type(aster_input::event_type_codes::EventTypes::REL);
        capability.set_supported_event_type(aster_input::event_type_codes::EventTypes::SYN);

        // Add mouse buttons.
        capability.set_supported_key(KeyCode::BtnLeft);
        capability.set_supported_key(KeyCode::BtnRight);
        capability.set_supported_key(KeyCode::BtnMiddle);

        // Add relative axes for movement.
        capability.set_supported_relative_axis(RelCode::X);
        capability.set_supported_relative_axis(RelCode::Y);
        capability.set_supported_relative_axis(RelCode::Wheel);

        Self {
            // Standard name for i8042 PS/2 mouse devices.
            name: "i8042_mouse".to_string(),

            // Physical path describing the device's connection topology
            // isa0060: ISA bus port 0x60 (i8042 data port)
            // serio1: Serial I/O device 1 (second PS/2 port)
            // input0: Input device 0 (first mouse device on this controller)
            phys: "isa0060/serio1/input0".to_string(),

            // Unique identifier - empty string because traditional i8042 mice
            // don't have unique hardware identifiers
            uniq: "".to_string(),

            // Device ID with standard values for i8042 mice
            // BUS_I8042 (0x11): PS/2 interface bus type
            // vendor (0x0002): Generic vendor ID for standard mice
            // product (0x0001): Generic product ID for standard mice
            // version (0x0001): Version 1.0 - standard PS/2 mouse protocol
            id: InputId::new(InputId::BUS_I8042, 0x0002, 0x0001, 0x0001),

            capability,
        }
    }
}

impl InputDevice for I8042Mouse {
    fn name(&self) -> &str {
        &self.name
    }

    fn phys(&self) -> &str {
        &self.phys
    }

    fn uniq(&self) -> &str {
        &self.uniq
    }

    fn id(&self) -> InputId {
        self.id
    }

    fn capability(&self) -> &InputCapability {
        &self.capability
    }
}

fn handle_mouse_input(_trap_frame: &TrapFrame) {
    if !I8042_CONTROLLER.is_completed() {
        return;
    }

    let Some(data) = read_mouse_data() else {
        return;
    };

    let mut packet_state = MOUSE_PACKET_STATE.lock();
    if let Some(events) = packet_state.process_byte(data) {
        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            registered_device.submit_events(&events);
        }
    }
}

/// Reads mouse data from the i8042 controller.
fn read_mouse_data() -> Option<u8> {
    let mut controller = I8042_CONTROLLER.get()?.lock();
    controller.receive_data()
}

/// Mouse packet state machine for handling 3-byte or 4-byte PS/2 mouse packets.
#[derive(Debug)]
struct MousePacketState {
    state: PacketState,
    packet: [u8; PACKET_LEN_INTELLIMOUSE],
    byte_index: usize,
    packet_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PacketState {
    WaitingForFirstByte,
    CollectingPacket,
}

impl MousePacketState {
    const fn new() -> Self {
        Self {
            state: PacketState::WaitingForFirstByte,
            packet: [0; PACKET_LEN_INTELLIMOUSE],
            byte_index: 0,
            packet_len: PACKET_LEN_STANDARD,
        }
    }

    fn set_packet_len(&mut self, len: usize) {
        self.packet_len = if len == 4 {
            PACKET_LEN_INTELLIMOUSE
        } else {
            PACKET_LEN_STANDARD
        };
        self.reset();
    }

    /// Processes a mouse data byte and returns input events if a complete packet is ready.
    fn process_byte(&mut self, data: u8) -> Option<Vec<InputEvent>> {
        match self.state {
            PacketState::WaitingForFirstByte => {
                // First byte must have bit 3 set (always 1 in PS/2 mouse packets).
                if data & PACKET_SYNC_MASK != 0 {
                    self.packet[0] = data;
                    self.byte_index = 1;
                    self.state = PacketState::CollectingPacket;
                }
                None
            }
            PacketState::CollectingPacket => {
                self.packet[self.byte_index] = data;
                self.byte_index += 1;

                if self.byte_index >= self.packet_len {
                    // Complete packet received.
                    let events = self.parse_packet();
                    self.reset();
                    Some(events)
                } else {
                    None
                }
            }
        }
    }

    /// Parses a complete PS/2 mouse packet and returns input events.
    fn parse_packet(&self) -> Vec<InputEvent> {
        let mut events = Vec::new();

        let status = self.packet[INDEX_STATUS];
        let x_delta = self.packet[INDEX_X] as i8;
        let y_delta = self.packet[INDEX_Y] as i8;

        let left_button = (status & BUTTON_LEFT_MASK) != 0;
        let right_button = (status & BUTTON_RIGHT_MASK) != 0;
        let middle_button = (status & BUTTON_MIDDLE_MASK) != 0;

        static PREV_LEFT: AtomicBool = AtomicBool::new(false);
        static PREV_RIGHT: AtomicBool = AtomicBool::new(false);
        static PREV_MIDDLE: AtomicBool = AtomicBool::new(false);

        let prev_left = PREV_LEFT.swap(left_button, Ordering::Relaxed);
        let prev_right = PREV_RIGHT.swap(right_button, Ordering::Relaxed);
        let prev_middle = PREV_MIDDLE.swap(middle_button, Ordering::Relaxed);

        if left_button != prev_left {
            events.push(InputEvent::key(
                KeyCode::BtnLeft,
                if left_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if right_button != prev_right {
            events.push(InputEvent::key(
                KeyCode::BtnRight,
                if right_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if middle_button != prev_middle {
            events.push(InputEvent::key(
                KeyCode::BtnMiddle,
                if middle_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if x_delta != 0 {
            events.push(InputEvent::relative(RelCode::X, x_delta as i32));
        }

        if y_delta != 0 {
            events.push(InputEvent::relative(RelCode::Y, -(y_delta as i32)));
        }

        if self.packet_len == PACKET_LEN_INTELLIMOUSE {
            let z_delta = self.packet[INDEX_WHEEL] as i8;
            if z_delta != 0 {
                events.push(InputEvent::relative(RelCode::Wheel, -(z_delta as i32)));
            }
        }

        // Add sync event to indicate end of this input report.
        if !events.is_empty() {
            events.push(InputEvent::sync(SynEvent::Report));
        }

        events
    }

    /// Resets the packet state machine to wait for the next packet.
    fn reset(&mut self) {
        self.state = PacketState::WaitingForFirstByte;
        self.byte_index = 0;
        self.packet = [0; PACKET_LEN_MAX];
    }
}
