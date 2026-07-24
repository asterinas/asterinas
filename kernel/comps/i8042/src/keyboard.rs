// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use alloc::{
    string::{String, ToString},
    sync::Arc,
};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{arch::device::io_port::ReadWriteAccess, io::IoPort};
use spin::Once;

use crate::{
    controller::{
        DATA_PORT_ADDR, I8042_CONTROLLER, I8042Controller, I8042ControllerError,
        STATUS_OR_COMMAND_PORT_ADDR,
    },
    ps2::{Command, CommandCtx},
};

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

pub(super) fn init(controller: &mut I8042Controller) -> Result<(), I8042ControllerError> {
    // Reference: <https://elixir.bootlin.com/linux/v6.17.9/source/drivers/input/serio/libps2.c#L184>
    const DEVICE_ID_REGULAR_KEYBOARD: u8 = 0xAB;

    let mut init_ctx = InitCtx(controller);

    init_ctx.reset()?;

    // Determine the keyboard's type.
    let (device_id, _) = init_ctx.get_device_id()?;
    ostd::info!("PS/2 keyboard device ID: 0x{:02X}", device_id);
    if device_id != DEVICE_ID_REGULAR_KEYBOARD {
        // TODO: Support other kinds of keyboards.
        return Err(I8042ControllerError::DeviceUnknown);
    }

    // Create and register the i8042 keyboard device.
    let keyboard_device = Arc::new(I8042Keyboard::new());
    let registered_device = aster_input::register_device(keyboard_device);
    REGISTERED_DEVICE.call_once(|| registered_device);

    Ok(())
}

struct InitCtx<'a>(&'a mut I8042Controller);

impl InitCtx<'_> {
    fn get_device_id(&mut self) -> Result<(u8, u8), I8042ControllerError> {
        let mut buf = [0u8; cmd::GetDeviceId::RES_LEN];
        self.command::<cmd::GetDeviceId>(&[], &mut buf)?;
        Ok((buf[0], buf[1]))
    }
}

impl CommandCtx for InitCtx<'_> {
    fn controller(&mut self) -> &mut I8042Controller {
        self.0
    }

    fn write_to_port(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        self.0.wait_and_send_data(data)
    }
}

mod cmd {
    use crate::ps2::{Command, define_commands};

    define_commands! {
        GetDeviceId, 0xF2, fn([u8; 0]) -> [u8; 2];
    }
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

/// Whether the Ctrl key is currently held.
///
/// Tracks the state of both LeftCtrl and RightCtrl. Used as a general-purpose modifier flag.
static CTRL_HELD: AtomicBool = AtomicBool::new(false);

/// Whether the Alt key is currently held.
///
/// Tracks the state of both LeftAlt and RightAlt. Used as a general-purpose modifier flag.
static ALT_HELD: AtomicBool = AtomicBool::new(false);

/// Dispatches keyboard input events to the input subsystem.
///
/// Called by the IRQ handler. Tracks modifier key state and detects key combinations
/// such as Ctrl+Alt+Del before dispatching events to the input subsystem.
pub(super) fn dispatch_keyboard_input() {
    let Some(scancode_event) = ScancodeInfo::read() else {
        return;
    };

    // Dispatch the input event.
    if let Some(key_code) = scancode_event.to_key_code() {
        // Track modifier key state.
        if matches!(key_code, KeyCode::LeftCtrl | KeyCode::RightCtrl) {
            CTRL_HELD.store(
                scancode_event.key_status == KeyStatus::Pressed,
                Ordering::Relaxed,
            );
        }
        if matches!(key_code, KeyCode::LeftAlt | KeyCode::RightAlt) {
            ALT_HELD.store(
                scancode_event.key_status == KeyStatus::Pressed,
                Ordering::Relaxed,
            );
        }

        // Detect Ctrl+Alt+Del and trigger a restart.
        if key_code == KeyCode::Delete
            && scancode_event.key_status == KeyStatus::Pressed
            && CTRL_HELD.load(Ordering::Relaxed)
            && ALT_HELD.load(Ordering::Relaxed)
        {
            ostd::warn!("Ctrl+Alt+Del detected, restarting the system");
            ostd::power::restart(ostd::power::ExitCode::Success);
        }

        // TODO: Implement virtual terminal switching for Alt+F1–F12 and
        // spawning a new console for Ctrl+Alt+F1–F12. For now we only log
        // the key combination and do not perform any VT switching.
        if scancode_event.key_status == KeyStatus::Pressed
            && let Some(n) = fn_key_number(key_code)
        {
            let ctrl = CTRL_HELD.load(Ordering::Relaxed);
            let alt = ALT_HELD.load(Ordering::Relaxed);
            match (ctrl, alt) {
                (true, true) => ostd::info!("Ctrl+Alt+F{} pressed", n),
                (false, true) => ostd::info!("Alt+F{} pressed", n),
                _ => {}
            }
        }

        if let Some(registered_device) = REGISTERED_DEVICE.get() {
            let events = [
                InputEvent::from_key_and_status(key_code, scancode_event.key_status),
                InputEvent::from_sync_event(SynEvent::Report),
            ];
            registered_device.submit_events(&events);
        }
    } else {
        ostd::debug!(
            "PS/2 keyboard unmapped scancode {:?} dropped",
            scancode_event.scancode
        );
    }
}

/// A scan code in the Scan Code Set 1.
///
/// Reference: <https://wiki.osdev.org/PS/2_Keyboard#Scan_Code_Set_1>.
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Debug)]
struct ScancodeInfo {
    scancode: ScanCode,
    key_status: KeyStatus,
    extended: bool,
}

impl ScancodeInfo {
    /// Reads the keyboard [`ScanCode`] from the i8042 controller.
    fn read() -> Option<Self> {
        static EXTENDED_KEY: AtomicBool = AtomicBool::new(false);

        let raw_data = if let Some(controller) = I8042_CONTROLLER.get() {
            controller.lock().receive_data()
        } else {
            read_scancode_from_data_port()
        }?;

        let data = if I8042_CONTROLLER.get().is_some() {
            raw_data
        } else {
            // When the i8042 controller is not available, we must perform software
            // Set 2 to Set 1 scancode translation.
            let is_extended_prefix = raw_data == 0xE0;
            let is_released = (raw_data & 0x80) != 0;
            let key = raw_data & 0x7F;
            if is_extended_prefix {
                // Extended scancode prefix (e.g. arrow keys, Insert, Delete).
                // Pass through as-is; the next byte will be translated.
                0xE0
            } else if key < 128 {
                // Normal key: translate Set 2 to Set 1 and preserve release bit.
                let translated = translate_set2_to_set1(key);
                if is_released {
                    translated | 0x80
                } else {
                    translated
                }
            } else {
                return None;
            }
        };

        let code = ScanCode(data);
        if code.has_error() {
            ostd::warn!("PS/2 keyboard key detection error or internal buffer overrun");
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

/// Returns the function-key number (1–12) if `key_code` is an F1–F12 key.
const fn fn_key_number(key_code: KeyCode) -> Option<u8> {
    match key_code {
        KeyCode::F1 => Some(1),
        KeyCode::F2 => Some(2),
        KeyCode::F3 => Some(3),
        KeyCode::F4 => Some(4),
        KeyCode::F5 => Some(5),
        KeyCode::F6 => Some(6),
        KeyCode::F7 => Some(7),
        KeyCode::F8 => Some(8),
        KeyCode::F9 => Some(9),
        KeyCode::F10 => Some(10),
        KeyCode::F11 => Some(11),
        KeyCode::F12 => Some(12),
        _ => None,
    }
}

/// Reads a single byte from the i8042 data port (0x60) directly.
///
/// Used as a fallback when the `I8042_CONTROLLER` singleton is unavailable.
/// The ports are temporarily acquired and released on each read so they can be reclaimed
/// if the controller is later initialized.
fn read_scancode_from_data_port() -> Option<u8> {
    const OUTPUT_BUFFER_FULL: u8 = 0x01;

    let Ok(status_port) = IoPort::<u8, ReadWriteAccess>::acquire(STATUS_OR_COMMAND_PORT_ADDR)
    else {
        return None;
    };

    // Check if there is data available in the output buffer.
    if status_port.read() & OUTPUT_BUFFER_FULL == 0 {
        return None;
    }

    let Ok(data_port) = IoPort::<u8, ReadWriteAccess>::acquire(DATA_PORT_ADDR) else {
        return None;
    };

    Some(data_port.read())
}

/// Software translation from Set 2 to Set 1 for 7-bit scancode values (0..127).
const fn translate_set2_to_set1(set2: u8) -> u8 {
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/drivers/input/keyboard/atkbd.c#L125>
    const SET2_TO_SET1: [u8; 128] = [
        0x00, 0x43, 0x41, 0x3F, 0x3D, 0x3B, 0x3C, 0x58, 0x64, 0x44, 0x42, 0x40, 0x3E, 0x0F, 0x29,
        0x59, 0x65, 0x38, 0x2A, 0x70, 0x1D, 0x10, 0x02, 0x5A, 0x66, 0x71, 0x2C, 0x1F, 0x1E, 0x11,
        0x03, 0x5B, 0x67, 0x2E, 0x2D, 0x20, 0x12, 0x05, 0x04, 0x5C, 0x68, 0x39, 0x2F, 0x21, 0x14,
        0x13, 0x06, 0x5D, 0x69, 0x31, 0x30, 0x23, 0x22, 0x15, 0x07, 0x5E, 0x6A, 0x72, 0x32, 0x24,
        0x16, 0x08, 0x09, 0x5F, 0x6B, 0x33, 0x25, 0x17, 0x18, 0x0B, 0x0A, 0x60, 0x6C, 0x34, 0x35,
        0x26, 0x27, 0x19, 0x0C, 0x61, 0x6D, 0x73, 0x28, 0x74, 0x1A, 0x0D, 0x62, 0x6E, 0x3A, 0x36,
        0x1C, 0x1B, 0x75, 0x2B, 0x63, 0x76, 0x55, 0x56, 0x77, 0x78, 0x79, 0x7A, 0x0E, 0x7B, 0x7C,
        0x4F, 0x7D, 0x4B, 0x47, 0x7E, 0x7F, 0x6F, 0x52, 0x53, 0x50, 0x4C, 0x4D, 0x48, 0x01, 0x45,
        0x57, 0x4E, 0x51, 0x4A, 0x37, 0x49, 0x46, 0x54,
    ];
    SET2_TO_SET1[set2 as usize]
}
