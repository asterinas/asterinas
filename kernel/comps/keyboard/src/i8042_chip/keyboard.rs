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
        kernel::{MappedIrqLine, IRQ_CHIP},
        trap::TrapFrame,
    },
    trap::irq::IrqLine,
};
use spin::Once;

use super::controller::{I8042Controller, I8042ControllerError, I8042_CONTROLLER};
use crate::alloc::string::ToString;

/// IRQ line for i8042 keyboard.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 keyboard.
const ISA_INTR_NUM: u8 = 1;

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Reset keyboard device by sending 0xFF (reset command, supported by all PS/2 devices) to port 1
    // and waiting for a response.
    controller.wait_and_send_data(0xFF)?;

    // The response should be 0xFA (ACK) and 0xAA (BAT successful), followed by the device PS/2 ID.
    if controller.wait_and_recv_data()? != 0xFA {
        return Err(I8042ControllerError::DeviceResetFailed);
    }
    // The reset command may take some time to finish. Try again a few times.
    if (0..5).find_map(|_| controller.wait_and_recv_data().ok()) != Some(0xAA) {
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

        // Adds all standard keyboard keys.
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

    let Some(scancode_event) = read_scancode() else {
        return;
    };

    // Dispatch the input event.
    if let Some(key_code) = scancode_event.to_key_code() {
        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            let events = [
                InputEvent::key(key_code, scancode_event.key_status),
                InputEvent::sync(SynEvent::SynReport),
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

impl ScancodeInfo {
    /// Maps the keyboard [`ScanCode`] to a [`KeyCode`] in the input subsystem.
    fn to_key_code(&self) -> Option<KeyCode> {
        // Remove the release bit.
        let code = self.scancode.0 & 0x7F;

        // Handle extended keys.
        if self.extended {
            return match code {
                0x47 => Some(KeyCode::KeyHome),
                0x48 => Some(KeyCode::KeyUp),
                0x49 => Some(KeyCode::KeyPageUp),
                0x4B => Some(KeyCode::KeyLeft),
                0x4D => Some(KeyCode::KeyRight),
                0x4F => Some(KeyCode::KeyEnd),
                0x50 => Some(KeyCode::KeyDown),
                0x51 => Some(KeyCode::KeyPageDown),
                0x52 => Some(KeyCode::KeyInsert),
                0x53 => Some(KeyCode::KeyDelete),
                _ => None,
            };
        }

        // Standard key mapping.
        Some(match code {
            // Letters - handle shift/caps lock
            0x1E => KeyCode::KeyA,
            0x30 => KeyCode::KeyB,
            0x2E => KeyCode::KeyC,
            0x20 => KeyCode::KeyD,
            0x12 => KeyCode::KeyE,
            0x21 => KeyCode::KeyF,
            0x22 => KeyCode::KeyG,
            0x23 => KeyCode::KeyH,
            0x17 => KeyCode::KeyI,
            0x24 => KeyCode::KeyJ,
            0x25 => KeyCode::KeyK,
            0x26 => KeyCode::KeyL,
            0x32 => KeyCode::KeyM,
            0x31 => KeyCode::KeyN,
            0x18 => KeyCode::KeyO,
            0x19 => KeyCode::KeyP,
            0x10 => KeyCode::KeyQ,
            0x13 => KeyCode::KeyR,
            0x1F => KeyCode::KeyS,
            0x14 => KeyCode::KeyT,
            0x16 => KeyCode::KeyU,
            0x2F => KeyCode::KeyV,
            0x11 => KeyCode::KeyW,
            0x2D => KeyCode::KeyX,
            0x15 => KeyCode::KeyY,
            0x2C => KeyCode::KeyZ,

            // Digits
            0x02 => KeyCode::Key1,
            0x03 => KeyCode::Key2,
            0x04 => KeyCode::Key3,
            0x05 => KeyCode::Key4,
            0x06 => KeyCode::Key5,
            0x07 => KeyCode::Key6,
            0x08 => KeyCode::Key7,
            0x09 => KeyCode::Key8,
            0x0A => KeyCode::Key9,
            0x0B => KeyCode::Key0,

            // Whitespace and control
            0x39 => KeyCode::KeySpace,
            0x0F => KeyCode::KeyTab,
            0x1C => KeyCode::KeyEnter,
            0x0E => KeyCode::KeyBackspace,

            // Punctuation
            0x0C => KeyCode::KeyMinus,
            0x0D => KeyCode::KeyEqual,
            0x29 => KeyCode::KeyGrave,
            0x2B => KeyCode::KeyBackslash,
            0x33 => KeyCode::KeyComma,
            0x34 => KeyCode::KeyDot,
            0x35 => KeyCode::KeySlash,
            0x27 => KeyCode::KeySemicolon,
            0x28 => KeyCode::KeyApostrophe,
            0x1A => KeyCode::KeyLeftBrace,
            0x1B => KeyCode::KeyRightBrace,

            // Modifier keys
            0x1D => KeyCode::KeyLeftCtrl,
            0x2A => KeyCode::KeyLeftShift,
            0x38 => KeyCode::KeyLeftAlt,
            0x3A => KeyCode::KeyCapsLock,

            // Function keys
            0x3B => KeyCode::KeyF1,
            0x3C => KeyCode::KeyF2,
            0x3D => KeyCode::KeyF3,
            0x3E => KeyCode::KeyF4,
            0x3F => KeyCode::KeyF5,
            0x40 => KeyCode::KeyF6,
            0x41 => KeyCode::KeyF7,
            0x42 => KeyCode::KeyF8,
            0x43 => KeyCode::KeyF9,
            0x44 => KeyCode::KeyF10,
            0x57 => KeyCode::KeyF11,
            0x58 => KeyCode::KeyF12,

            // Escape
            0x01 => KeyCode::KeyEsc,

            // Unhandled mappings -> None
            _ => return None,
        })
    }
}

/// A scan code in the Scan Code Set 1.
///
/// Reference: <https://wiki.osdev.org/PS/2_Keyboard#Scan_Code_Set_1>.
#[derive(Debug, Clone, Copy)]
struct ScanCode(u8);

impl ScanCode {
    fn has_error(&self) -> bool {
        // Key detection error or internal buffer overrun.
        self.0 == 0xFF
    }

    fn key_status(&self) -> KeyStatus {
        if self.0 & 0x80 == 0 {
            KeyStatus::Pressed
        } else {
            KeyStatus::Released
        }
    }

    fn is_extension(&self) -> bool {
        self.0 == 0xE0
    }
}

/// Scancode value and relevant modifier states.
#[derive(Debug, Clone)]
struct ScancodeInfo {
    scancode: ScanCode,
    key_status: KeyStatus,
    extended: bool,
}

fn read_scancode() -> Option<ScancodeInfo> {
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

    Some(ScancodeInfo {
        scancode: code,
        key_status,
        extended,
    })
}
