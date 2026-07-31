// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use alloc::{
    string::{String, ToString},
    sync::Arc,
};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus, SynEvent},
    input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice},
};
use ostd::{
    arch::{
        irq::{IRQ_CHIP, MappedIrqLine},
        trap::TrapFrame,
    },
    irq::IrqLine,
};
use spin::Once;

use crate::{
    controller::{I8042_CONTROLLER, I8042Controller, I8042ControllerError},
    ps2::{Command, CommandCtx},
};

/// IRQ line for i8042 keyboard.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

/// Registered device instance for event submission.
static REGISTERED_DEVICE: Once<RegisteredInputDevice> = Once::new();

/// ISA interrupt number for i8042 keyboard.
const ISA_INTR_NUM: u8 = 1;

/// The current translation mode for keyboard scancodes.
static TRANSLATION_MODE: AtomicU8 = AtomicU8::new(TranslationMode::Hardware as u8);

/// How Scan Code Set 2 scancodes are translated to Set 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum TranslationMode {
    /// The i8042 controller translates Set 2 to Set 1 in hardware.
    Hardware = 0,
    /// No hardware translation; the driver translates Set 2 to Set 1 in software.
    Software = 1,
}

impl TranslationMode {
    fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Hardware,
            _ => Self::Software,
        }
    }
}

pub(super) fn init(
    controller: &mut I8042Controller,
    translation_mode: TranslationMode,
) -> Result<(), I8042ControllerError> {
    // Reference: <https://elixir.bootlin.com/linux/v6.17.9/source/drivers/input/serio/libps2.c#L184>
    const DEVICE_ID_REGULAR_KEYBOARD: u8 = 0xAB;

    TRANSLATION_MODE.store(translation_mode as u8, Ordering::Relaxed);

    let mut init_ctx = InitCtx(controller);

    if ATKBD_RESET.load(Ordering::Relaxed) {
        // Linux treats a failed keyboard reset as non-fatal — it just warns and continues.
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/drivers/input/keyboard/atkbd.c#L820>
        if let Err(err) = init_ctx.reset() {
            ostd::warn!("PS/2 keyboard reset failed: {:?}", err);
        }
    }

    // If the get ID command fails, Linux falls back to SETLEDS as a probe — if that succeeds,
    // the device is still treated as a valid keyboard (return 0). We take a simpler approach:
    // just tolerate the failure and continue.
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/drivers/input/keyboard/atkbd.c#L838>
    match init_ctx.get_device_id() {
        Ok((device_id, _)) => {
            ostd::info!("PS/2 keyboard device ID: 0x{:02X}", device_id);
            if device_id != DEVICE_ID_REGULAR_KEYBOARD {
                // TODO: Support other kinds of keyboards.
                return Err(I8042ControllerError::DeviceUnknown);
            }
        }
        Err(err) => {
            ostd::warn!("PS/2 keyboard device ID query failed ({:?})", err);
        }
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

fn handle_keyboard_input(_trap_frame: &TrapFrame) {
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
        ostd::debug!(
            "PS/2 keyboard unmapped scancode {:?} dropped",
            scancode_event.scancode
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct ScanCode(u8);

impl ScanCode {
    const RELEASE_MASK: u8 = 0x80;

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
    /// Reads the keyboard scancode from the i8042 controller.
    fn read() -> Option<Self> {
        let Some(data) = I8042_CONTROLLER.get()?.lock().receive_data() else {
            ostd::warn!("PS/2 keyboard has no input data");
            return None;
        };

        let mode = TranslationMode::from_u8(TRANSLATION_MODE.load(Ordering::Relaxed));
        if mode == TranslationMode::Hardware {
            return Self::read_set1(data);
        }
        Self::read_set2(data)
    }

    /// Interprets a byte as a Scan Code Set 1 scancode (hardware translation enabled).
    fn read_set1(data: u8) -> Option<Self> {
        static EXTENDED_KEY: AtomicBool = AtomicBool::new(false);

        let code = ScanCodeSet1(data);
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
        let extended = EXTENDED_KEY.swap(false, Ordering::Relaxed);

        Some(Self {
            scancode: ScanCode(code.0),
            key_status,
            extended,
        })
    }

    /// Interprets a byte as a Scan Code Set 2 scancode (no hardware translation).
    fn read_set2(data: u8) -> Option<Self> {
        static EXTENDED_KEY: AtomicBool = AtomicBool::new(false);
        static RELEASE_NEXT: AtomicBool = AtomicBool::new(false);

        let code = ScanCodeSet2(data);

        if code.has_error() {
            ostd::warn!("PS/2 keyboard key detection error or internal buffer overrun");
            return None;
        }

        if code.is_extension() {
            EXTENDED_KEY.store(true, Ordering::Relaxed);
            return None;
        }

        if code.is_release_prefix() {
            RELEASE_NEXT.store(true, Ordering::Relaxed);
            return None;
        }

        let releasing = RELEASE_NEXT.swap(false, Ordering::Relaxed);
        let extended = EXTENDED_KEY.swap(false, Ordering::Relaxed);

        let set1_code = SET2_TO_SET1[code.raw() as usize];
        if set1_code == 0xFF {
            return None;
        }

        let key_status = if releasing {
            KeyStatus::Released
        } else {
            KeyStatus::Pressed
        };

        Some(Self {
            scancode: ScanCode(if releasing {
                set1_code | 0x80
            } else {
                set1_code
            }),
            key_status,
            extended,
        })
    }

    /// Maps the keyboard scancode to a [`KeyCode`] in the input subsystem.
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

/// A raw scan code byte in Scan Code Set 1.
///
/// Reference: <https://wiki.osdev.org/PS/2_Keyboard#Scan_Code_Set_1>.
#[derive(Clone, Copy, Debug)]
struct ScanCodeSet1(u8);

impl ScanCodeSet1 {
    const CODE_ERROR: u8 = 0xFF;
    const CODE_EXT_PREFIX: u8 = 0xE0;
    const RELEASE_MASK: u8 = 0x80;

    fn has_error(&self) -> bool {
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
}

/// A raw scan code byte in Scan Code Set 2.
///
/// Reference: <https://wiki.osdev.org/PS/2_Keyboard#Scan_Code_Set_2>.
#[derive(Clone, Copy, Debug)]
struct ScanCodeSet2(u8);

impl ScanCodeSet2 {
    const CODE_ERROR: u8 = 0xFF;
    const CODE_EXT_PREFIX: u8 = 0xE0;
    const CODE_RELEASE_PREFIX: u8 = 0xF0;

    fn has_error(&self) -> bool {
        self.0 == Self::CODE_ERROR
    }

    fn is_extension(&self) -> bool {
        self.0 == Self::CODE_EXT_PREFIX
    }

    fn is_release_prefix(&self) -> bool {
        self.0 == Self::CODE_RELEASE_PREFIX
    }

    fn raw(&self) -> u8 {
        self.0
    }
}

/// Whether to send the keyboard reset command during initialization.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/drivers/input/keyboard/atkbd.c#L40>
static ATKBD_RESET: AtomicBool = AtomicBool::new(cfg_select! {
    any(target_arch = "x86_64", target_arch = "loongarch64") => false,
    _ => true,
});
aster_cmdline::define_flag_param!("atkbd.reset", ATKBD_RESET);

/// Scan Code Set 2 to Set 1 translation table.
///
/// Reference: <https://www.win.tue.nl/~aeb/linux/kbd/scancodes-10.html#ss10.3>
const SET2_TO_SET1: [u8; 256] = [
    // 0x00-0x0F
    0xff, 0x43, 0x41, 0x3f, 0x3d, 0x3b, 0x3c, 0x58, 0x64, 0x44, 0x42, 0x40, 0x3e, 0x0f, 0x29, 0x59,
    // 0x10-0x1F
    0x65, 0x38, 0x2a, 0x70, 0x1d, 0x10, 0x02, 0x5a, 0x66, 0x71, 0x2c, 0x1f, 0x1e, 0x11, 0x03, 0x5b,
    // 0x20-0x2F
    0x67, 0x2e, 0x2d, 0x20, 0x12, 0x05, 0x04, 0x5c, 0x68, 0x39, 0x2f, 0x21, 0x14, 0x13, 0x06, 0x5d,
    // 0x30-0x3F
    0x69, 0x31, 0x30, 0x23, 0x22, 0x15, 0x07, 0x5e, 0x6a, 0x72, 0x32, 0x24, 0x16, 0x08, 0x09, 0x5f,
    // 0x40-0x4F
    0x6b, 0x33, 0x25, 0x17, 0x18, 0x0b, 0x0a, 0x60, 0x6c, 0x34, 0x35, 0x26, 0x27, 0x19, 0x0c, 0x61,
    // 0x50-0x5F
    0x6d, 0x73, 0x28, 0x74, 0x1a, 0x0d, 0x62, 0x6e, 0x3a, 0x36, 0x1c, 0x1b, 0x75, 0x2b, 0x63, 0x76,
    // 0x60-0x6F
    0x55, 0x56, 0x77, 0x78, 0x79, 0x7a, 0x0e, 0x7b, 0x7c, 0x4f, 0x7d, 0x4b, 0x47, 0x7e, 0x7f, 0x6f,
    // 0x70-0x7F
    0x52, 0x53, 0x50, 0x4c, 0x4d, 0x48, 0x01, 0x45, 0x57, 0x4e, 0x51, 0x4a, 0x37, 0x49, 0x46, 0x54,
    // 0x80-0x8F
    0x80, 0x81, 0x82, 0x41, 0x54, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
    // 0x90-0x9F
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
    // 0xA0-0xAF
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
    // 0xB0-0xBF
    0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf,
    // 0xC0-0xCF
    0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcc, 0xcd, 0xce, 0xcf,
    // 0xD0-0xDF
    0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xdb, 0xdc, 0xdd, 0xde, 0xdf,
    // 0xE0-0xEF
    0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef,
    // 0xF0-0xFF
    0xff, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];
