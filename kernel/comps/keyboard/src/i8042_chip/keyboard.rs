// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use alloc::{string::String, sync::Arc};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, SynEvent},
    InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice,
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
use crate::{alloc::string::ToString, InputKey, KEYBOARD_CALLBACKS};

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

    // Creates and registers the i8042 keyboard device.
    let keyboard_device = Arc::new(I8042Keyboard::new());
    let registered_device = aster_input::register_device(keyboard_device);

    REGISTERED_DEVICE.call_once(|| registered_device);

    Ok(())
}

#[derive(Debug)]
pub struct I8042Keyboard {
    name: String,
    phys: String,
    uniq: String,
    id: InputId,
    capability: InputCapability,
}

impl I8042Keyboard {
    pub fn new() -> Self {
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

    let Some(keyboard_state) = parse_inputkey() else {
        return;
    };

    // Dispatches the input event.
    // Map `ScanCode` directly to Linux KeyCode; drop if unsupported
    if let Some(linux_key) = scancode_to_key_code(&keyboard_state) {
        // Send key press/release event
        let key_event = InputEvent::key(linux_key, keyboard_state.key_status);

        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            registered_device.submit_event(&key_event);

            // Send synchronization event
            let syn_event = InputEvent::sync(SynEvent::SynReport);
            registered_device.submit_event(&syn_event);
        } else {
            log::error!("Keyboard: REGISTERED_DEVICE not found! Event dropped!");
        }
    } else {
        log::debug!(
            "Keyboard: unmapped scancode {:?}, dropped",
            keyboard_state.scancode
        );
    }

    if keyboard_state.key_status == KeyStatus::Pressed {
        if let Some(input_key) = scancode_to_input_key(&keyboard_state) {
            for callback in KEYBOARD_CALLBACKS.lock().iter() {
                callback(input_key);
            }
        }
    }
}

/// Map `ScanCode` directly to Linux `KeyCode`.
fn scancode_to_key_code(keyboard_state: &KeyboardState) -> Option<KeyCode> {
    // Removes the release bit.
    let code = keyboard_state.scancode.0 & 0x7F;

    // Handles extended keys.
    if keyboard_state.extended {
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

/// Map `ScanCode` to `InputKey`
fn scancode_to_input_key(keyboard_state: &KeyboardState) -> Option<InputKey> {
    // Removes the release bit.
    let code = keyboard_state.scancode.0 & 0x7F;

    // Handles extended keys.
    if keyboard_state.extended {
        return match code {
            0x47 => Some(InputKey::Home),
            0x48 => Some(InputKey::UpArrow),
            0x49 => Some(InputKey::PageUp),
            0x4B => Some(InputKey::LeftArrow),
            0x4D => Some(InputKey::RightArrow),
            0x4F => Some(InputKey::End),
            0x50 => Some(InputKey::DownArrow),
            0x51 => Some(InputKey::PageDown),
            0x52 => Some(InputKey::Insert),
            0x53 => Some(InputKey::Delete),
            _ => None,
        };
    }

    // Handles modifier keys: they don't generate `InputKey` events.
    if keyboard_state.scancode.is_ctrl()
        || keyboard_state.scancode.is_shift()
        || keyboard_state.scancode.is_caps_lock()
    {
        return None;
    }

    // Applies modifier states to get the correct `InputKey`.
    let ctrl_key = keyboard_state.ctrl_key;
    let shift_key = keyboard_state.shift_key;
    let caps_lock = keyboard_state.caps_lock;

    // Determines which mapping to use based on modifier states.
    let key = if ctrl_key {
        // Ctrl mapping
        match code {
            0x02 => InputKey::One,
            0x03 => InputKey::Nul,
            0x04 => InputKey::Esc,
            0x05 => InputKey::Fs,
            0x06 => InputKey::Gs,
            0x07 => InputKey::Rs,
            0x08 => InputKey::Us,
            0x09 => InputKey::Del,
            0x0A => InputKey::Nine,
            0x0B => InputKey::Zero,
            0x0C => InputKey::Us,
            0x0D => InputKey::Equal,
            0x0E => InputKey::Bs,
            0x10 => InputKey::Dc1,
            0x11 => InputKey::Etb,
            0x12 => InputKey::Enq,
            0x13 => InputKey::Dc2,
            0x14 => InputKey::Dc4,
            0x15 => InputKey::Em,
            0x16 => InputKey::Nak,
            0x17 => InputKey::Tab,
            0x18 => InputKey::Si,
            0x19 => InputKey::Dle,
            0x1A => InputKey::Esc,
            0x1B => InputKey::Gs,
            0x1C => InputKey::Cr,
            0x1E => InputKey::Soh,
            0x1F => InputKey::Dc3,
            0x20 => InputKey::Eot,
            0x21 => InputKey::Ack,
            0x22 => InputKey::Bel,
            0x23 => InputKey::Bs,
            0x24 => InputKey::Lf,
            0x25 => InputKey::Vt,
            0x26 => InputKey::Ff,
            0x27 => InputKey::SemiColon,
            0x28 => InputKey::SingleQuote,
            0x29 => InputKey::Backtick,
            0x2B => InputKey::Fs,
            0x2C => InputKey::Sub,
            0x2D => InputKey::Can,
            0x2E => InputKey::Etx,
            0x2F => InputKey::Syn,
            0x30 => InputKey::Stx,
            0x31 => InputKey::So,
            0x32 => InputKey::Cr,
            0x33 => InputKey::Comma,
            0x34 => InputKey::Period,
            0x35 => InputKey::Us,
            _ => return None,
        }
    } else if shift_key ^ caps_lock {
        // Shift or CapsLock mapping
        match code {
            0x01 => InputKey::Esc,
            0x02 => InputKey::Exclamation,
            0x03 => InputKey::At,
            0x04 => InputKey::Hash,
            0x05 => InputKey::Dollar,
            0x06 => InputKey::Percent,
            0x07 => InputKey::Caret,
            0x08 => InputKey::Ampersand,
            0x09 => InputKey::Asterisk,
            0x0A => InputKey::LeftParen,
            0x0B => InputKey::RightParen,
            0x0C => InputKey::Underscore,
            0x0D => InputKey::Plus,
            0x0E => InputKey::Del,
            0x0F => InputKey::Tab,
            0x10 => InputKey::UppercaseQ,
            0x11 => InputKey::UppercaseW,
            0x12 => InputKey::UppercaseE,
            0x13 => InputKey::UppercaseR,
            0x14 => InputKey::UppercaseT,
            0x15 => InputKey::UppercaseY,
            0x16 => InputKey::UppercaseU,
            0x17 => InputKey::UppercaseI,
            0x18 => InputKey::UppercaseO,
            0x19 => InputKey::UppercaseP,
            0x1A => InputKey::LeftBrace,
            0x1B => InputKey::RightBrace,
            0x1C => InputKey::Cr,
            0x1E => InputKey::UppercaseA,
            0x1F => InputKey::UppercaseS,
            0x20 => InputKey::UppercaseD,
            0x21 => InputKey::UppercaseF,
            0x22 => InputKey::UppercaseG,
            0x23 => InputKey::UppercaseH,
            0x24 => InputKey::UppercaseJ,
            0x25 => InputKey::UppercaseK,
            0x26 => InputKey::UppercaseL,
            0x27 => InputKey::Colon,
            0x28 => InputKey::DoubleQuote,
            0x29 => InputKey::Tilde,
            0x2B => InputKey::Pipe,
            0x2C => InputKey::UppercaseZ,
            0x2D => InputKey::UppercaseX,
            0x2E => InputKey::UppercaseC,
            0x2F => InputKey::UppercaseV,
            0x30 => InputKey::UppercaseB,
            0x31 => InputKey::UppercaseN,
            0x32 => InputKey::UppercaseM,
            0x33 => InputKey::LessThan,
            0x34 => InputKey::GreaterThan,
            0x35 => InputKey::Question,
            0x39 => InputKey::Space,
            _ => return None,
        }
    } else {
        // Plain mapping
        match code {
            0x01 => InputKey::Esc,
            0x02 => InputKey::One,
            0x03 => InputKey::Two,
            0x04 => InputKey::Three,
            0x05 => InputKey::Four,
            0x06 => InputKey::Five,
            0x07 => InputKey::Six,
            0x08 => InputKey::Seven,
            0x09 => InputKey::Eight,
            0x0A => InputKey::Nine,
            0x0B => InputKey::Zero,
            0x0C => InputKey::Minus,
            0x0D => InputKey::Equal,
            0x0E => InputKey::Del,
            0x0F => InputKey::Tab,
            0x10 => InputKey::LowercaseQ,
            0x11 => InputKey::LowercaseW,
            0x12 => InputKey::LowercaseE,
            0x13 => InputKey::LowercaseR,
            0x14 => InputKey::LowercaseT,
            0x15 => InputKey::LowercaseY,
            0x16 => InputKey::LowercaseU,
            0x17 => InputKey::LowercaseI,
            0x18 => InputKey::LowercaseO,
            0x19 => InputKey::LowercaseP,
            0x1A => InputKey::LeftBracket,
            0x1B => InputKey::RightBracket,
            0x1C => InputKey::Cr, // Enter
            0x1D => return None,  // Left Ctrl
            0x1E => InputKey::LowercaseA,
            0x1F => InputKey::LowercaseS,
            0x20 => InputKey::LowercaseD,
            0x21 => InputKey::LowercaseF,
            0x22 => InputKey::LowercaseG,
            0x23 => InputKey::LowercaseH,
            0x24 => InputKey::LowercaseJ,
            0x25 => InputKey::LowercaseK,
            0x26 => InputKey::LowercaseL,
            0x27 => InputKey::SemiColon,
            0x28 => InputKey::SingleQuote,
            0x29 => InputKey::Backtick,
            0x2A => return None, // Left Shift
            0x2B => InputKey::BackSlash,
            0x2C => InputKey::LowercaseZ,
            0x2D => InputKey::LowercaseX,
            0x2E => InputKey::LowercaseC,
            0x2F => InputKey::LowercaseV,
            0x30 => InputKey::LowercaseB,
            0x31 => InputKey::LowercaseN,
            0x32 => InputKey::LowercaseM,
            0x33 => InputKey::Comma,
            0x34 => InputKey::Period,
            0x35 => InputKey::ForwardSlash,
            0x36 => return None, // Right Shift
            0x37 => InputKey::Asterisk,
            0x38 => return None, // Left Alt
            0x39 => InputKey::Space,
            0x3A => return None, // CapsLock
            0x3B => InputKey::F1,
            0x3C => InputKey::F2,
            0x3D => InputKey::F3,
            0x3E => InputKey::F4,
            0x3F => InputKey::F5,
            0x40 => InputKey::F6,
            0x41 => InputKey::F7,
            0x42 => InputKey::F8,
            0x43 => InputKey::F9,
            0x44 => InputKey::F10,
            0x45 => return None, // NumLock
            0x46 => return None, // ScrollLock
            0x47 => InputKey::Home,
            0x48 => InputKey::UpArrow,
            0x49 => InputKey::PageUp,
            0x4A => InputKey::Minus,
            0x4B => InputKey::LeftArrow,
            0x4C => InputKey::Five,
            0x4D => InputKey::RightArrow,
            0x4E => InputKey::Plus,
            0x4F => InputKey::End,
            0x50 => InputKey::DownArrow,
            0x51 => InputKey::PageDown,
            0x52 => InputKey::Insert,
            0x53 => InputKey::Delete,
            0x57 => InputKey::F11,
            0x58 => InputKey::F12,
            _ => return None,
        }
    };

    Some(key)
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

    fn is_shift(&self) -> bool {
        let code = self.0 & 0x7F;
        // Left/right shift codes
        code == 0x2A || code == 0x36
    }

    fn is_ctrl(&self) -> bool {
        let code = self.0 & 0x7F;
        // Left/right ctrl codes
        code == 0x1D
    }

    fn is_caps_lock(&self) -> bool {
        self.0 == 0x3A
    }

    fn is_extension(&self) -> bool {
        self.0 == 0xE0
    }
}

/// Keyboard state information.
#[derive(Debug, Clone)]
struct KeyboardState {
    scancode: ScanCode,
    key_status: KeyStatus,
    caps_lock: bool,
    shift_key: bool,
    ctrl_key: bool,
    extended: bool,
}

fn parse_inputkey() -> Option<KeyboardState> {
    static CAPS_LOCK: AtomicBool = AtomicBool::new(false); // CapsLock key state
    static SHIFT_KEY: AtomicBool = AtomicBool::new(false); // Shift key pressed
    static CTRL_KEY: AtomicBool = AtomicBool::new(false); // Ctrl key pressed
    static EXTENDED_KEY: AtomicBool = AtomicBool::new(false); // Extended key flag

    let Some(data) = I8042_CONTROLLER.get().unwrap().lock().receive_data() else {
        log::warn!("i8042 keyboard has no input data");
        return None;
    };

    let code = ScanCode(data);
    if code.has_error() {
        log::warn!("i8042 keyboard key detection error or internal buffer overrun");
        return None;
    }

    // Handles the extension code.
    if code.is_extension() {
        EXTENDED_KEY.store(true, Ordering::Relaxed);
        return None;
    }

    let key_status = code.key_status();
    let caps_lock = CAPS_LOCK.load(Ordering::Relaxed);
    let shift_key = SHIFT_KEY.load(Ordering::Relaxed);
    let ctrl_key = CTRL_KEY.load(Ordering::Relaxed);
    let extended = EXTENDED_KEY.load(Ordering::Relaxed);

    // Handles the Ctrl key, holds the state.
    if code.is_ctrl() {
        if key_status == KeyStatus::Pressed {
            CTRL_KEY.store(true, Ordering::Relaxed);
        } else {
            CTRL_KEY.store(false, Ordering::Relaxed);
        }
    }

    // Handles the Shift key, holds the state.
    if code.is_shift() {
        if key_status == KeyStatus::Pressed {
            SHIFT_KEY.store(true, Ordering::Relaxed);
        } else {
            SHIFT_KEY.store(false, Ordering::Relaxed);
        }
    }

    // Handles the CapsLock key, flips the state.
    if code.is_caps_lock() && key_status == KeyStatus::Pressed {
        CAPS_LOCK.fetch_xor(true, Ordering::Relaxed);
    }

    // Clears extended flag if this is not an extended key.
    if extended {
        EXTENDED_KEY.store(false, Ordering::Relaxed);
    }

    // Returns the complete keyboard state.
    Some(KeyboardState {
        scancode: code,
        key_status,
        caps_lock,
        shift_key,
        ctrl_key,
        extended,
    })
}
