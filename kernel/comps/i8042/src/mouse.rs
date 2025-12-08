// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, RelCode, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{
    arch::{
        irq::{IRQ_CHIP, MappedIrqLine},
        trap::TrapFrame,
    },
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

use crate::{
    controller::{I8042_CONTROLLER, I8042Controller, I8042ControllerError},
    ps2::{Command, CommandCtx},
};

/// IRQ line for i8042 mouse.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 mouse.
const ISA_INTR_NUM: u8 = 12;

/// Mouse packet state machine.
static PACKET_STATE: SpinLock<PacketState, LocalIrqDisabled> = SpinLock::new(PacketState::new());

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    let mut init_ctx = InitCtx(controller);

    let device_id = init_ctx
        .reset()?
        .ok_or(I8042ControllerError::DeviceUnknown)?;
    log::info!("PS/2 mouse device ID: 0x{:02X}", device_id);

    // Determine the mouse's type.
    let mut mouse_type =
        MouseType::from_device_id(device_id).ok_or(I8042ControllerError::DeviceUnknown)?;
    if mouse_type == MouseType::Standard && init_ctx.enable_intellimouse().is_ok() {
        // Query the device ID again.
        let new_device_id = init_ctx.get_device_id()?;
        log::info!("PS/2 mouse upgraded device ID: 0x{:02X}", new_device_id);
        mouse_type =
            MouseType::from_device_id(new_device_id).ok_or(I8042ControllerError::DeviceUnknown)?;
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

    // Configure the mouse's type based on its device ID.
    let mut state = PACKET_STATE.lock();
    state.set_mouse_type(mouse_type);

    Ok(())
}

struct InitCtx<'a>(&'a mut I8042Controller);

impl InitCtx<'_> {
    fn get_device_id(&mut self) -> Result<u8, I8042ControllerError> {
        let mut buf = [0u8; cmd::GetDeviceId::RES_LEN];
        self.command::<cmd::GetDeviceId>(&[], &mut buf)?;
        Ok(buf[0])
    }

    fn enable_intellimouse(&mut self) -> Result<(), I8042ControllerError> {
        const SAMPLE_RATE_200: u8 = 200;
        const SAMPLE_RATE_100: u8 = 100;
        const SAMPLE_RATE_80: u8 = 80;

        // Set the sample rate to 200, then to 100, and finally to 80.
        // Reference: <https://wiki.osdev.org/Mouse_Input#Init/Detection_Command_Sequences>
        self.set_sample_rate(SAMPLE_RATE_200)?;
        self.set_sample_rate(SAMPLE_RATE_100)?;
        self.set_sample_rate(SAMPLE_RATE_80)?;
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u8) -> Result<(), I8042ControllerError> {
        let mut buf = [0u8; cmd::SetSampleRate::RES_LEN];
        self.command::<cmd::SetSampleRate>(&[rate], &mut buf)?;
        Ok(())
    }

    fn enable_data_reporting(&mut self) -> Result<(), I8042ControllerError> {
        let mut buf = [0u8; cmd::EnableDataReporting::RES_LEN];
        self.command::<cmd::EnableDataReporting>(&[], &mut buf)?;
        Ok(())
    }
}

impl CommandCtx for InitCtx<'_> {
    fn controller(&mut self) -> &mut I8042Controller {
        self.0
    }

    fn write_to_port(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        self.0.write_to_second_port(data)
    }
}

mod cmd {
    use crate::ps2::{Command, define_commands};

    define_commands! {
        GetDeviceId, 0xF2, fn([u8; 0]) -> [u8; 1];
        SetSampleRate, 0xF3, fn([u8; 1]) -> [u8; 0];
        EnableDataReporting, 0xF4, fn([u8; 0]) -> [u8; 0];
    }
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
    let Some(controller) = I8042_CONTROLLER.get() else {
        return;
    };
    let Some(data) = controller.lock().receive_data() else {
        log::warn!("PS/2 mouse has no input data");
        return;
    };

    let mut packet_state = PACKET_STATE.lock();
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
    const PACKET_LEN_STANDARD: usize = 3;
    const PACKET_LEN_INTELLIMOUSE: usize = 4;
    const PACKET_LEN_MAX: usize = 4;

    fn from_device_id(device_id: u8) -> Option<Self> {
        // Reference: <https://wiki.osdev.org/Mouse_Input#MouseID_Byte>
        const DEVICE_ID_STANDARD_MOUSE: u8 = 0x00;
        const DEVICE_ID_INTELLIMOUSE: u8 = 0x03;
        const DEVICE_ID_INTELLIMOUSE_EXPLORER: u8 = 0x04;

        match device_id {
            DEVICE_ID_STANDARD_MOUSE => Some(MouseType::Standard),
            DEVICE_ID_INTELLIMOUSE | DEVICE_ID_INTELLIMOUSE_EXPLORER => {
                Some(MouseType::IntelliMouse)
            }
            _ => None,
        }
    }

    fn packet_len(&self) -> usize {
        match self {
            MouseType::Standard => Self::PACKET_LEN_STANDARD,
            MouseType::IntelliMouse => Self::PACKET_LEN_INTELLIMOUSE,
        }
    }
}

/// Mouse packet state machine for handling 3-byte or 4-byte PS/2 mouse packets.
struct PacketState {
    packet: [u8; MouseType::PACKET_LEN_MAX],
    byte_index: usize,
    mouse_type: MouseType,
    prev_left: bool,
    prev_right: bool,
    prev_middle: bool,
}

impl PacketState {
    const fn new() -> Self {
        Self {
            packet: [0; MouseType::PACKET_LEN_MAX],
            byte_index: 0,
            mouse_type: MouseType::Standard,
            prev_left: false,
            prev_right: false,
            prev_middle: false,
        }
    }

    fn set_mouse_type(&mut self, mouse_type: MouseType) {
        self.mouse_type = mouse_type;
        self.reset();
    }

    /// Processes a mouse data byte and returns input events if a complete packet is ready.
    fn process_byte(&mut self, data: u8) -> Option<Vec<InputEvent>> {
        const PACKET_SYNC_MASK: u8 = 0x08;

        // The first byte must have bit 3 set (always 1 in PS/2 mouse packets).
        if self.byte_index == 0 && (data & PACKET_SYNC_MASK) == 0 {
            return None;
        }

        // Collect the byte into the packet.
        self.packet[self.byte_index] = data;
        self.byte_index += 1;
        if self.byte_index < self.mouse_type.packet_len() {
            return None;
        }

        // A complete packet has been received.
        let events = self.parse_packet();
        self.reset();
        Some(events)
    }

    /// Parses a complete PS/2 mouse packet and returns input events.
    ///
    /// Reference: <https://wiki.osdev.org/Mouse_Input#Format_of_First_3_Packet_Bytes>.
    fn parse_packet(&mut self) -> Vec<InputEvent> {
        const BUTTON_LEFT_MASK: u8 = 0x01;
        const BUTTON_RIGHT_MASK: u8 = 0x02;
        const BUTTON_MIDDLE_MASK: u8 = 0x04;
        const X_SIGN_MASK: u8 = 0x10;
        const Y_SIGN_MASK: u8 = 0x20;
        const OVERFLOWED_MASK: u8 = 0xC0;
        const INDEX_STATUS: usize = 0;
        const INDEX_X: usize = 1;
        const INDEX_Y: usize = 2;
        const INDEX_WHEEL: usize = 3;

        // Currently, this method can generate at most 7 events. Don't forget to update this when
        // modifying the logic below!
        let mut events = Vec::with_capacity(7);

        let status = self.packet[INDEX_STATUS];
        if (status & OVERFLOWED_MASK) != 0 {
            log::warn!("PS/2 mouse packet overflowed");
            return events;
        }

        let x_delta = add_sign_bit(self.packet[INDEX_X], (status & X_SIGN_MASK) != 0);
        let y_delta = add_sign_bit(self.packet[INDEX_Y], (status & Y_SIGN_MASK) != 0);

        let left_button = (status & BUTTON_LEFT_MASK) != 0;
        let right_button = (status & BUTTON_RIGHT_MASK) != 0;
        let middle_button = (status & BUTTON_MIDDLE_MASK) != 0;

        let prev_left = self.prev_left;
        let prev_right = self.prev_right;
        let prev_middle = self.prev_middle;
        self.prev_left = left_button;
        self.prev_right = right_button;
        self.prev_middle = middle_button;

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
            events.push(InputEvent::from_relative_move(RelCode::X, x_delta));
        }

        if y_delta != 0 {
            events.push(InputEvent::from_relative_move(RelCode::Y, -y_delta));
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

        // Add a sync event to indicate end of this input report.
        if !events.is_empty() {
            events.push(InputEvent::from_sync_event(SynEvent::Report));
        }

        events
    }

    /// Resets the packet state machine to wait for the next packet.
    fn reset(&mut self) {
        self.byte_index = 0;
    }
}

/// Extends a byte to a signed 32-bit integer using the sign bit from the status byte.
fn add_sign_bit(byte: u8, sign_bit: bool) -> i32 {
    let mut byte = byte as u32;
    if sign_bit {
        byte |= 0xFFFFFF00;
    }
    byte as i32
}
