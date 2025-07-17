// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::key::KeyStatus;
use ostd::{
    arch::{
        kernel::{MappedIrqLine, IRQ_CHIP},
        trap::TrapFrame,
    },
    trap::irq::IrqLine,
};
use spin::Once;

use super::controller::{I8042Controller, I8042ControllerError, I8042_CONTROLLER};
use crate::{InputKey, KEYBOARD_CALLBACKS};

/// IRQ line for i8042 keyboard.
static IRQ_LINE: Once<MappedIrqLine> = Once::new();

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

    Ok(())
}

fn handle_keyboard_input(_trap_frame: &TrapFrame) {
    if !I8042_CONTROLLER.is_completed() {
        return;
    }

    let Some((key, KeyStatus::Pressed)) = parse_inputkey() else {
        // Only trigger callbacks for key press events.
        return;
    };
    for callback in KEYBOARD_CALLBACKS.lock().iter() {
        callback(key);
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

    fn extended_map(&self) -> Option<InputKey> {
        let key = match self.0 & 0x7F {
            0x1D => return None, // Right Ctrl
            0x38 => return None, // Right Alt
            0x47 => InputKey::Home,
            0x48 => InputKey::UpArrow,
            0x49 => InputKey::PageUp,
            0x4B => InputKey::LeftArrow,
            0x4D => InputKey::RightArrow,
            0x4F => InputKey::End,
            0x50 => InputKey::DownArrow,
            0x51 => InputKey::PageDown,
            0x52 => InputKey::Insert,
            0x53 => InputKey::Delete,
            _ => return None,
        };
        Some(key)
    }

    fn plain_map(&self) -> Option<InputKey> {
        let key = match self.0 & 0x7F {
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
            0x36 => return None,        // Right Shift
            0x37 => InputKey::Asterisk, // Keypad-* or (*/PrtScn) on a 83/84-key keyboard
            0x38 => return None,        // Left Alt
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
            0x45 => return None,          // NumLock
            0x46 => return None,          // ScrollLock
            0x47 => InputKey::Home,       // Keypad-7 or Home
            0x48 => InputKey::UpArrow,    // Keypad-8 or Up
            0x49 => InputKey::PageUp,     // Keypad-9 or PageUp
            0x4A => InputKey::Minus,      // Keypad--
            0x4B => InputKey::LeftArrow,  // Keypad-4 or Left
            0x4C => InputKey::Five,       // Keypad-5
            0x4D => InputKey::RightArrow, // Keypad-6 or Right
            0x4E => InputKey::Plus,       // Keypad-+
            0x4F => InputKey::End,        // Keypad-1 or End
            0x50 => InputKey::DownArrow,  // Keypad-2 or Down
            0x51 => InputKey::PageDown,   // Keypad-3 or PageDown
            0x52 => InputKey::Insert,     // Keypad-0 or Insert
            0x53 => InputKey::Delete,     // Keypad-. or Del
            0x57 => InputKey::F11,
            0x58 => InputKey::F12,
            _ => return None,
        };
        Some(key)
    }

    fn shift_map(&self) -> Option<InputKey> {
        let key = match self.0 & 0x7F {
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
        };
        Some(key)
    }

    fn ctrl_map(&self) -> Option<InputKey> {
        let key = match self.0 & 0x7F {
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
        };
        Some(key)
    }
}

fn parse_inputkey() -> Option<(InputKey, KeyStatus)> {
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

    // Handle the extension code.
    if code.is_extension() {
        EXTENDED_KEY.store(true, Ordering::Relaxed);
        return None;
    }

    let is_extended = EXTENDED_KEY.load(Ordering::Relaxed);
    if is_extended {
        EXTENDED_KEY.store(false, Ordering::Relaxed);
        return code.extended_map().map(|k| (k, code.key_status()));
    }

    let key_status = code.key_status();

    // Handle the Ctrl key, holds the state.
    if code.is_ctrl() {
        if key_status == KeyStatus::Pressed {
            CTRL_KEY.store(true, Ordering::Relaxed);
        } else {
            CTRL_KEY.store(false, Ordering::Relaxed);
        }
        return None;
    }

    // Handle the Shift key, holds the state.
    if code.is_shift() {
        if key_status == KeyStatus::Pressed {
            SHIFT_KEY.store(true, Ordering::Relaxed);
        } else {
            SHIFT_KEY.store(false, Ordering::Relaxed);
        }
        return None;
    }

    // Handle the CapsLock key, flips the state.
    if code.is_caps_lock() {
        if key_status == KeyStatus::Pressed {
            CAPS_LOCK.fetch_xor(true, Ordering::Relaxed);
        }
        return None;
    }

    let ctrl_key = CTRL_KEY.load(Ordering::Relaxed);
    let shift_key = SHIFT_KEY.load(Ordering::Relaxed);
    let caps_lock = CAPS_LOCK.load(Ordering::Relaxed);
    let key = if ctrl_key {
        code.ctrl_map()
    } else if shift_key ^ caps_lock {
        code.shift_map()
    } else {
        code.plain_map()
    };

    key.map(|k| (k, key_status))
}
