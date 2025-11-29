// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use alloc::{string::String, sync::Arc};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{
    arch::{
        irq::{MappedIrqLine, IRQ_CHIP},
        trap::TrapFrame,
    },
    irq::IrqLine,
};
use spin::Once;

use super::controller::{
    I8042Controller, I8042ControllerError, I8042_CONTROLLER, PS2_ACK, PS2_BAT_OK, PS2_CMD_RESET,
};
use crate::{alloc::string::ToString, controller::PS2_RESULTS};

/// IRQ line for i8042 keyboard.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 keyboard.
const ISA_INTR_NUM: u8 = 1;

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Reset the keyboard device by sending `PS2_CMD_RESET` (reset command, supported by all PS/2
    // devices) to port 1 and waiting for a response.
    controller.wait_and_send_data(PS2_CMD_RESET)?;

    // The response should be `PS2_ACK` and `PS2_BAT_OK`, followed by the device PS/2 ID.
    if controller.wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // The reset command may take some time to finish.
    if controller.wait_long_and_recv_data()? != PS2_BAT_OK {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // See <https://wiki.osdev.org/I8042_PS/2_Controller#Detecting_PS/2_Device_Types> for a list of IDs.
    let mut iter = core::iter::from_fn(|| controller.wait_and_recv_data().ok());
    match (iter.next(), iter.next()) {
        // Ancient AT keyboard
        (None, None) => (),
        // Other devices, including other kinds of keyboards (TODO: Support other kinds of keyboards)
        _ => return Err(I8042ControllerError::DeviceUnknown),
    }

    let mut irq_line = IrqLine::alloc()
        .and_then(|irq_line| {
            IRQ_CHIP
                .get()
                .unwrap()
                .map_isa_pin_to(irq_line, ISA_INTR_NUM)
        })
        .map_err(|_| I8042ControllerError::DeviceAllocIrqFailed)?;
    irq_line.on_active(handle_keyboard_input);
    IRQ_LINE.call_once(|| irq_line);

    // Create and register the i8042 keyboard device.
    let keyboard_device = Arc::new(I8042Keyboard::new());
    let registered_device = aster_input::register_device(keyboard_device);
    REGISTERED_DEVICE.call_once(|| registered_device);

    Ok(())
}

#[derive(Debug)]
struct I8042Keyboard {
    name: String,
    phys: String,
    uniq: String,
    id: InputId,
    capability: InputCapability,
}

impl I8042Keyboard {
    fn new() -> Self {
        let mut capability = InputCapability::new();

        capability.set_supported_event_type(aster_input::event_type_codes::EventTypes::KEY);
        capability.set_supported_event_type(aster_input::event_type_codes::EventTypes::SYN);

        // Add all standard keyboard keys.
        capability.add_standard_keyboard_keys();

        Self {
            // Standard name for i8042 PS/2 keyboard devices.
            name: "i8042_keyboard".to_string(),

            // Physical path describing the device's connection topology
            // isa0060: ISA bus port 0x60 (i8042 data port)
            // serio0: Serial I/O device 0 (PS/2 is a serial protocol)
            // input0: Input device 0 (first input device on this controller)
            phys: "isa0060/serio0/input0".to_string(),

            // Unique identifier - empty string because traditional i8042 keyboards
            // don't have unique hardware identifiers like serial numbers or MAC addresses
            uniq: "".to_string(),

            // Device ID with standard values for i8042 keyboards
            // BUS_I8042 (0x11): PS/2 interface bus type
            // vendor (0x0001): Generic vendor ID for standard keyboards
            // product (0x0001): Generic product ID for standard keyboards
            // version (0x0001): Version 1.0 - standard PS/2 keyboard protocol
            id: InputId::new(InputId::BUS_I8042, 0x0001, 0x0001, 0x0001),

            capability,
        }
    }
}

impl InputDevice for I8042Keyboard {
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

fn handle_keyboard_input(_trap_frame: &TrapFrame) {
    if !I8042_CONTROLLER.is_completed() {
        return;
    }

    let Some(scancode_event) = ScancodeInfo::read() else {
        return;
    };

    // Dispatch the input event.
    if let Some(key_code) = scancode_event.to_key_code() {
        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            let events = [
                InputEvent::from_key_and_status(key_code, scancode_event.key_status),
                InputEvent::from_sync_event(SynEvent::Report),
            ];
            registered_device.submit_events(&events);
        }
    } else {
        log::debug!(
            "i8042 keyboard unmapped scancode {:?} dropped",
            scancode_event.scancode
        );
    }
}

/// A scan code in the Scan Code Set 1.
///
/// Reference: <https://wiki.osdev.org/PS/2_Keyboard#Scan_Code_Set_1>.
#[derive(Debug, Clone, Copy)]
struct ScanCode(u8);

impl ScanCode {
    const CODE_ERROR: u8 = 0xFF;
    const CODE_EXT_PREFIX: u8 = 0xE0;
    const RELEASE_MASK: u8 = 0x80;

    fn has_error(&self) -> bool {
        // Key detection error or internal buffer overrun.
        self.0 == Self::CODE_ERROR
    }

    fn key_status(&self) -> KeyStatus {
        if self.0 & Self::RELEASE_MASK == 0 {
            KeyStatus::Pressed
        } else {
            KeyStatus::Released
        }
    }

    fn is_extension(&self) -> bool {
        self.0 == Self::CODE_EXT_PREFIX
    }

    fn key(&self) -> u8 {
        self.0 & !Self::RELEASE_MASK
    }
}

/// Scancode value and relevant modifier states.
#[derive(Debug, Clone)]
struct ScancodeInfo {
    scancode: ScanCode,
    key_status: KeyStatus,
    extended: bool,
}

impl ScancodeInfo {
    /// Reads the keyboard [`ScanCode`] from the i8042 controller.
    fn read() -> Option<Self> {
        static EXTENDED_KEY: AtomicBool = AtomicBool::new(false);

        let Some(data) = I8042_CONTROLLER.get().unwrap().lock().receive_data() else {
            log::warn!("i8042 keyboard has no input data");
            return None;
        };

        let code = ScanCode(data);
        if code.has_error() {
            log::warn!("i8042 keyboard key detection error or internal buffer overrun");
            return None;
        }

        // Handle the extension code.
        if code.is_extension() {
            EXTENDED_KEY.store(true, Ordering::Relaxed);
            return None;
        }

        let key_status = code.key_status();
        let extended = EXTENDED_KEY.load(Ordering::Relaxed);

        // Clear extended flag if this is not an extended key.
        if extended {
            EXTENDED_KEY.store(false, Ordering::Relaxed);
        }

        Some(Self {
            scancode: code,
            key_status,
            extended,
        })
    }

    /// Maps the keyboard [`ScanCode`] to a [`KeyCode`] in the input subsystem.
    fn to_key_code(&self) -> Option<KeyCode> {
        // Remove the release bit.
        let code = self.scancode.key();

        // Handle extended keys.
        if self.extended {
            return match code {
                0x47 => Some(KeyCode::Home),
                0x48 => Some(KeyCode::Up),
                0x49 => Some(KeyCode::PageUp),
                0x4B => Some(KeyCode::Left),
                0x4D => Some(KeyCode::Right),
                0x4F => Some(KeyCode::End),
                0x50 => Some(KeyCode::Down),
                0x51 => Some(KeyCode::PageDown),
                0x52 => Some(KeyCode::Insert),
                0x53 => Some(KeyCode::Delete),
                _ => None,
            };
        }

        // Standard key mapping.
        Some(match code {
            // Letters - handle shift/caps lock
            0x1E => KeyCode::A,
            0x30 => KeyCode::B,
            0x2E => KeyCode::C,
            0x20 => KeyCode::D,
            0x12 => KeyCode::E,
            0x21 => KeyCode::F,
            0x22 => KeyCode::G,
            0x23 => KeyCode::H,
            0x17 => KeyCode::I,
            0x24 => KeyCode::J,
            0x25 => KeyCode::K,
            0x26 => KeyCode::L,
            0x32 => KeyCode::M,
            0x31 => KeyCode::N,
            0x18 => KeyCode::O,
            0x19 => KeyCode::P,
            0x10 => KeyCode::Q,
            0x13 => KeyCode::R,
            0x1F => KeyCode::S,
            0x14 => KeyCode::T,
            0x16 => KeyCode::U,
            0x2F => KeyCode::V,
            0x11 => KeyCode::W,
            0x2D => KeyCode::X,
            0x15 => KeyCode::Y,
            0x2C => KeyCode::Z,

            // Digits
            0x02 => KeyCode::Num1,
            0x03 => KeyCode::Num2,
            0x04 => KeyCode::Num3,
            0x05 => KeyCode::Num4,
            0x06 => KeyCode::Num5,
            0x07 => KeyCode::Num6,
            0x08 => KeyCode::Num7,
            0x09 => KeyCode::Num8,
            0x0A => KeyCode::Num9,
            0x0B => KeyCode::Num0,

            // Whitespace and control
            0x39 => KeyCode::Space,
            0x0F => KeyCode::Tab,
            0x1C => KeyCode::Enter,
            0x0E => KeyCode::Backspace,

            // Punctuation
            0x0C => KeyCode::Minus,
            0x0D => KeyCode::Equal,
            0x29 => KeyCode::Grave,
            0x2B => KeyCode::Backslash,
            0x33 => KeyCode::Comma,
            0x34 => KeyCode::Dot,
            0x35 => KeyCode::Slash,
            0x27 => KeyCode::Semicolon,
            0x28 => KeyCode::Apostrophe,
            0x1A => KeyCode::LeftBrace,
            0x1B => KeyCode::RightBrace,

            // Modifier keys
            0x1D => KeyCode::LeftCtrl,
            0x2A => KeyCode::LeftShift,
            0x38 => KeyCode::LeftAlt,
            0x3A => KeyCode::CapsLock,

            // Function keys
            0x3B => KeyCode::F1,
            0x3C => KeyCode::F2,
            0x3D => KeyCode::F3,
            0x3E => KeyCode::F4,
            0x3F => KeyCode::F5,
            0x40 => KeyCode::F6,
            0x41 => KeyCode::F7,
            0x42 => KeyCode::F8,
            0x43 => KeyCode::F9,
            0x44 => KeyCode::F10,
            0x57 => KeyCode::F11,
            0x58 => KeyCode::F12,

            // Escape
            0x01 => KeyCode::Esc,

            // Unhandled mappings -> None
            _ => return None,
        })
    }
}
