// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{
    array,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, RelCode, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{
    arch::{
        irq::{MappedIrqLine, IRQ_CHIP},
        trap::TrapFrame,
    },
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

use super::controller::{
    I8042Controller, I8042ControllerError, I8042_CONTROLLER, PS2_ACK, PS2_BAT_OK, PS2_CMD_RESET,
};
use crate::alloc::string::ToString;

const PACKET_LEN_STANDARD: usize = 3;
const PACKET_LEN_INTELLIMOUSE: usize = 4;
const PACKET_LEN_MAX: usize = 4;

/// IRQ line for i8042 mouse.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 mouse.
const ISA_INTR_NUM: u8 = 12;

/// Mouse packet state machine.
static MOUSE_PACKET_STATE: SpinLock<MousePacketState, LocalIrqDisabled> =
    SpinLock::new(MousePacketState::new());

trait MouseCommand {
    const CMD_BYTE: u8;
    const DATA_LEN: usize;
    const RES_LEN: usize;
}

mod mouse_cmd {
    use super::MouseCommand;

    macro_rules! define_commands {
        (
            $(
                $name:ident, $cmd:literal, fn([u8; $dlen:literal]) -> [u8; $rlen:literal];
            )*
        ) => {
            $(
                pub(super) struct $name;
                impl MouseCommand for $name {
                    const CMD_BYTE: u8 = $cmd;
                    const DATA_LEN: usize = $dlen;
                    const RES_LEN: usize = $rlen;
                }
            )*
        };
    }

    define_commands! {
        GetDeviceId, 0xF2, fn([u8; 0]) -> [u8; 1];
        SetSampleRate, 0xF3, fn([u8; 1]) -> [u8; 0];
        EnableDataReporting, 0xF4, fn([u8; 0]) -> [u8; 0];
    }
}
use mouse_cmd::*;

struct MouseInitCtx<'a>(&'a mut I8042Controller);

impl MouseInitCtx<'_> {
    fn command<C: MouseCommand>(
        &mut self,
        args: &[u8],
    ) -> Result<[u8; C::RES_LEN], I8042ControllerError> {
        debug_assert_eq!(args.len(), C::DATA_LEN);

        self.0.write_to_second_port(C::CMD_BYTE)?;
        if self.0.wait_and_recv_data()? != PS2_ACK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }

        for &arg in args {
            self.0.write_to_second_port(arg)?;
            if self.0.wait_and_recv_data()? != PS2_ACK {
                return Err(I8042ControllerError::DeviceResetFailed);
            }
        }

        array::try_from_fn(|_| self.0.wait_and_recv_data())
            .map_err(|_| I8042ControllerError::DeviceResetFailed)
    }

    fn get_device_id(&mut self) -> Result<u8, I8042ControllerError> {
        let [device_id] = self.command::<GetDeviceId>(&[])?;
        Ok(device_id)
    }

    fn set_sample_rate(&mut self, rate: u8) -> Result<(), I8042ControllerError> {
        self.command::<SetSampleRate>(&[rate])?;
        Ok(())
    }

    fn enable_data_reporting(&mut self) -> Result<(), I8042ControllerError> {
        self.command::<EnableDataReporting>(&[])?;
        Ok(())
    }

    fn enable_intellimouse(&mut self) -> Result<(), I8042ControllerError> {
        const SAMPLE_RATE_200: u8 = 200;
        const SAMPLE_RATE_100: u8 = 100;
        const SAMPLE_RATE_80: u8 = 80;

        // Set sample rate with 200, then 100, then 80.
        self.set_sample_rate(SAMPLE_RATE_200)?;
        self.set_sample_rate(SAMPLE_RATE_100)?;
        self.set_sample_rate(SAMPLE_RATE_80)?;
        Ok(())
    }
}

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Reset mouse device by sending `PS2_CMD_RESET` to the second PS/2 port.
    controller.write_to_second_port(PS2_CMD_RESET)?;

    // The response should be `PS2_ACK` and `PS2_BAT_OK`, followed by the device PS/2 ID.
    if controller.wait_and_recv_data()? != PS2_ACK {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // The reset command may take some time to finish. Try again a few times.
    if (0..5).find_map(|_| controller.wait_and_recv_data().ok()) != Some(PS2_BAT_OK) {
        return Err(I8042ControllerError::DeviceResetFailed);
    }

    let mut device_id = controller
        .wait_and_recv_data()
        .map_err(|_| I8042ControllerError::DeviceUnknown)?;
    log::info!("PS/2 mouse device ID: 0x{:02X}", device_id);

    let mut init_ctx = MouseInitCtx(controller);

    // Try to enable IntelliMouse.
    // Reference: https://wiki.osdev.org/Mouse_Input
    if init_ctx.enable_intellimouse().is_ok() {
        // Query device ID again.
        if let Ok(new_id) = init_ctx.get_device_id() {
            device_id = new_id;
            log::info!("PS/2 mouse upgraded device ID: 0x{:02X}", device_id);
        }
    }

    // Enable data reporting.
    init_ctx.enable_data_reporting()?;

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

    // Configure mouse type based on device ID.
    if let Some(mouse_type) = MouseType::from_device_id(device_id) {
        let mut state = MOUSE_PACKET_STATE.lock();
        state.set_mouse_type(mouse_type);
    }

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

    let Some(controller) = I8042_CONTROLLER.get() else {
        return;
    };
    let Some(data) = controller.lock().receive_data() else {
        return;
    };

    let mut packet_state = MOUSE_PACKET_STATE.lock();
    if let Some(events) = packet_state.process_byte(data) {
        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            registered_device.submit_events(&events);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum MouseType {
    Standard,
    IntelliMouse,
}

impl MouseType {
    fn from_device_id(device_id: u8) -> Option<Self> {
        const DEVICE_ID_INTELLIMOUSE: u8 = 0x03;
        const DEVICE_ID_INTELLIMOUSE_EXPLORER: u8 = 0x04;

        match device_id {
            DEVICE_ID_INTELLIMOUSE | DEVICE_ID_INTELLIMOUSE_EXPLORER => {
                Some(MouseType::IntelliMouse)
            }
            _ => Some(MouseType::Standard),
        }
    }

    fn packet_len(&self) -> usize {
        match self {
            MouseType::Standard => PACKET_LEN_STANDARD,
            MouseType::IntelliMouse => PACKET_LEN_INTELLIMOUSE,
        }
    }
}

/// Mouse packet state machine for handling 3-byte or 4-byte PS/2 mouse packets.
struct MousePacketState {
    state: PacketState,
    packet: [u8; PACKET_LEN_INTELLIMOUSE],
    byte_index: usize,
    mouse_type: MouseType,
    prev_left: AtomicBool,
    prev_right: AtomicBool,
    prev_middle: AtomicBool,
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
            mouse_type: MouseType::Standard,
            prev_left: AtomicBool::new(false),
            prev_right: AtomicBool::new(false),
            prev_middle: AtomicBool::new(false),
        }
    }

    fn set_mouse_type(&mut self, mouse_type: MouseType) {
        self.mouse_type = mouse_type;
        self.reset();
    }

    /// Processes a mouse data byte and returns input events if a complete packet is ready.
    fn process_byte(&mut self, data: u8) -> Option<Vec<InputEvent>> {
        const PACKET_SYNC_MASK: u8 = 0x08;

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

                if self.byte_index >= self.mouse_type.packet_len() {
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
    fn parse_packet(&mut self) -> Vec<InputEvent> {
        const BUTTON_LEFT_MASK: u8 = 0x01;
        const BUTTON_RIGHT_MASK: u8 = 0x02;
        const BUTTON_MIDDLE_MASK: u8 = 0x04;
        const INDEX_STATUS: usize = 0;
        const INDEX_X: usize = 1;
        const INDEX_Y: usize = 2;
        const INDEX_WHEEL: usize = 3;

        let mut events = Vec::new();

        let status = self.packet[INDEX_STATUS];
        let x_delta = self.packet[INDEX_X] as i8;
        let y_delta = self.packet[INDEX_Y] as i8;

        let left_button = (status & BUTTON_LEFT_MASK) != 0;
        let right_button = (status & BUTTON_RIGHT_MASK) != 0;
        let middle_button = (status & BUTTON_MIDDLE_MASK) != 0;

        let prev_left = self.prev_left.swap(left_button, Ordering::Relaxed);
        let prev_right = self.prev_right.swap(right_button, Ordering::Relaxed);
        let prev_middle = self.prev_middle.swap(middle_button, Ordering::Relaxed);

        if left_button != prev_left {
            events.push(InputEvent::from_key_and_status(
                KeyCode::BtnLeft,
                if left_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if right_button != prev_right {
            events.push(InputEvent::from_key_and_status(
                KeyCode::BtnRight,
                if right_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if middle_button != prev_middle {
            events.push(InputEvent::from_key_and_status(
                KeyCode::BtnMiddle,
                if middle_button {
                    KeyStatus::Pressed
                } else {
                    KeyStatus::Released
                },
            ));
        }

        if x_delta != 0 {
            events.push(InputEvent::from_relative_move(RelCode::X, x_delta as i32));
        }

        if y_delta != 0 {
            events.push(InputEvent::from_relative_move(
                RelCode::Y,
                -(y_delta as i32),
            ));
        }

        if self.mouse_type == MouseType::IntelliMouse {
            let z_delta = self.packet[INDEX_WHEEL] as i8;
            if z_delta != 0 {
                events.push(InputEvent::from_relative_move(
                    RelCode::Wheel,
                    -(z_delta as i32),
                ));
            }
        }

        // Add sync event to indicate end of this input report.
        if !events.is_empty() {
            events.push(InputEvent::from_sync_event(SynEvent::Report));
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
