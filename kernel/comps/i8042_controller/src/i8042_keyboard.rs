// SPDX-License-Identifier: MPL-2.0

//! The i8042 keyboard driver.

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{
    arch::{device::io_port::ReadWriteAccess},
    io::IoPort,
    sync::SpinLock,
    trap::{IrqLine, TrapFrame},
};
use spin::Once;
use alloc::sync::Arc;
use aster_input::{InputDevice, InputDeviceMeta, InputEvent, input_event};
use aster_time::tsc::read_instant;

use crate::alloc::string::ToString;
use super::{InputKey, KEYBOARD_CALLBACKS};

use crate::DATA_PORT;
use crate::STATUS_PORT;
use crate::KEYBOARD_IRQ_LINE;


pub fn init() {
    log::error!("This is init in kernel/comps/keyboard/src/i8042_keyboard.rs");
    aster_input::register_device("i8042_keyboard".to_string(), Arc::new(I8042Keyboard));
}
struct I8042Keyboard;

impl InputDevice for I8042Keyboard {
    fn metadata(&self) -> InputDeviceMeta {
        InputDeviceMeta {
            name: "i8042_keyboard".to_string(),
            vendor_id: 0x1234,    // Replace with the actual vendor ID
            product_id: 0x5678,  // Replace with the actual product ID
            version: 1,          // Replace with the actual version
        }
    }
}

pub fn handle_keyboard_input(_trap_frame: &TrapFrame) {
    log::error!("-----This is handle_keyboard_input in kernel/comps/i8042_controller/src/i8042_keyboard.rs");
    let key = parse_inputkey();

    // Get the current time in microseconds
    let now = read_instant();
    let time_in_microseconds = now.secs() * 1_000_000 + (now.nanos() / 1_000) as u64;

    // Dispatch the input event
    input_event(InputEvent {
        time: time_in_microseconds, // Assign the current timestamp
        type_: 1,                   // EV_KEY (example type for key events)
        code: key as u16,           // Convert InputKey to a u16 representation
        value: 1,                   // Example value (1 for key press, 0 for release)
    }, "i8042_keyboard");

    // Fixme: the callbacks are going to be replaced.
    for callback in KEYBOARD_CALLBACKS.lock().iter() {
        callback(key);
    }
}

#[derive(Debug, Clone, Copy)]
struct ScanCode(u8);

impl ScanCode {
    fn read() -> Self {
        Self(DATA_PORT.get().unwrap().read())
    }

    fn is_valid(&self) -> bool {
        self.0 != 0xFF
    }

    fn is_pressed(&self) -> bool {
        self.0 & 0x80 == 0
    }

    fn is_released(&self) -> bool {
        self.0 & 0x80 != 0
    }

    fn is_shift(&self) -> bool {
        let code = self.0 & 0x7F;
        /* Left/right shift */
        code == 0x2A || code == 0x36
    }

    fn is_ctrl(&self) -> bool {
        let code = self.0 & 0x7F;
        /* Left/right ctrl */
        code == 0x1D || code == 0x61
    }

    fn is_caps_lock(&self) -> bool {
        self.0 == 0x3A
    }

    fn is_extension(&self) -> bool {
        self.0 == 0xE0
    }

    fn plain_map(&self) -> InputKey {
        match self.0 & 0x7F {
            0x00 => InputKey::Nul,
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
            0x1C => InputKey::Cr,  // Enter
            0x1D => InputKey::Nul, // Left Ctrl
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
            0x2A => InputKey::Nul, // Left Shift
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
            0x36 => InputKey::Nul,      // Right Shift
            0x37 => InputKey::Asterisk, // Keypad-* or (*/PrtScn) on a 83/84-key keyboard
            0x38 => InputKey::Nul,      // Left Alt
            0x39 => InputKey::Space,
            0x3A => InputKey::Nul, // CapsLock
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
            0x45 => InputKey::Nul,        // NumLock
            0x46 => InputKey::Nul,        // ScrollLock
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
            _ => InputKey::Nul,
        }
    }

    fn shift_map(&self) -> InputKey {
        match self.0 & 0x7F {
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
            _ => InputKey::Nul,
        }
    }

    fn ctrl_map(&self) -> InputKey {
        match self.0 & 0x7F {
            0x02 => InputKey::One,
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
            _ => InputKey::Nul,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Status(u8);

impl Status {
    const STAT_OUTPUT_BUFFER_FULL: u8 = 0x01; /* Keyboard output buffer full */

    fn read() -> Self {
        Self(STATUS_PORT.get().unwrap().read())
    }

    fn is_valid(&self) -> bool {
        self.0 != 0xFF
    }

    fn output_buffer_is_full(&self) -> bool {
        self.0 & Self::STAT_OUTPUT_BUFFER_FULL == 1
    }
}

fn parse_inputkey() -> InputKey {
    static CAPS_LOCK: AtomicBool = AtomicBool::new(false); /* CapsLock state (0-off, 1-on) */
    static SHIFT_KEY: AtomicBool = AtomicBool::new(false); /* Shift next keypress */
    static CTRL_KEY: AtomicBool = AtomicBool::new(false);

    let code = ScanCode::read();
    let status = Status::read();

    if !code.is_valid() || !status.is_valid() {
        log::debug!("i8042 keyboard does not exist");
        return InputKey::Nul;
    }

    if status.output_buffer_is_full() {
        log::debug!("i8042 keyboard output buffer full");
    }

    /* Skip extension code */
    if code.is_extension() {
        return InputKey::Nul;
    }

    if code.is_ctrl() {
        if code.is_pressed() {
            CTRL_KEY.store(true, Ordering::Relaxed);
        } else {
            CTRL_KEY.store(false, Ordering::Relaxed);
        }
        return InputKey::Nul;
    }

    // Ignore release, trigger on make, except for shift keys, where
    // we want to keep the shift state so long as the key is held down.
    if code.is_shift() {
        if code.is_pressed() {
            SHIFT_KEY.store(true, Ordering::Relaxed);
        } else {
            SHIFT_KEY.store(false, Ordering::Relaxed);
        }
        return InputKey::Nul;
    }

    /* Ignore other release events */
    if code.is_released() {
        return InputKey::Nul;
    }

    if code.is_pressed() && code.is_caps_lock() {
        CAPS_LOCK.fetch_xor(true, Ordering::Relaxed);
        return InputKey::Nul;
    }

    let ctrl_key = CTRL_KEY.load(Ordering::Relaxed);
    let shift_key = SHIFT_KEY.load(Ordering::Relaxed);
    let caps_lock = CAPS_LOCK.load(Ordering::Relaxed);
    if ctrl_key {
        code.ctrl_map()
    } else if shift_key || caps_lock {
        code.shift_map()
    } else {
        code.plain_map()
    }
}
