// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use component::{init_component, ComponentInitError};
use ostd::{
    arch::x86::device::keyboard::{self, KEYBOARD_DATA_PORT, KEYBOARD_STATUS_PORT},
    sync::SpinLock,
    trap::TrapFrame,
};

static KEYBOARD_CALLBACKS: SpinLock<Vec<Box<KeyboardCallback>>> = SpinLock::new(Vec::new());

/// The callback function for keyboard.
pub type KeyboardCallback = dyn Fn(Key) + Send + Sync;

#[derive(Debug, Clone, Copy)]
pub enum Key {
    Home,
    End,
    Up,
    Down,
    Left,
    Right,
    Enter,
    BackSpace,
    Delete,
    Escape,
    Char(char),
    Fn(u8),
    Ctrl(char),
    Alt(char),
    Null,
}

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    keyboard::register_callback(handle_keyboard_input);
    Ok(())
}

pub fn register_callback(callback: &'static KeyboardCallback) {
    KEYBOARD_CALLBACKS
        .disable_irq()
        .lock()
        .push(Box::new(callback));
}

fn handle_keyboard_input(_trap_frame: &TrapFrame) {
    let key = keyboard_getkey();
    for callback in KEYBOARD_CALLBACKS.lock().iter() {
        callback(key);
    }
}

#[derive(Debug, Clone, Copy)]
struct KeyCode(u8);

impl KeyCode {
    const NR_KEYS: usize = 128;
    const PLAIN_MAP: [u16; Self::NR_KEYS] = [
        0xf200, 0xf01b, 0xf031, 0xf032, 0xf033, 0xf034, 0xf035, 0xf036, 0xf037, 0xf038, 0xf039,
        0xf030, 0xf02d, 0xf03d, 0xf07f, 0xf009, 0xfb71, 0xfb77, 0xfb65, 0xfb72, 0xfb74, 0xfb79,
        0xfb75, 0xfb69, 0xfb6f, 0xfb70, 0xf05b, 0xf05d, 0xf201, 0xf702, 0xfb61, 0xfb73, 0xfb64,
        0xfb66, 0xfb67, 0xfb68, 0xfb6a, 0xfb6b, 0xfb6c, 0xf03b, 0xf027, 0xf060, 0xf700, 0xf05c,
        0xfb7a, 0xfb78, 0xfb63, 0xfb76, 0xfb62, 0xfb6e, 0xfb6d, 0xf02c, 0xf02e, 0xf02f, 0xf700,
        0xf30c, 0xf703, 0xf020, 0xf207, 0xf100, 0xf101, 0xf102, 0xf103, 0xf104, 0xf105, 0xf106,
        0xf107, 0xf108, 0xf109, 0xf208, 0xf209, 0xf307, 0xf308, 0xf309, 0xf30b, 0xf304, 0xf305,
        0xf306, 0xf30a, 0xf301, 0xf302, 0xf303, 0xf300, 0xf310, 0xf206, 0xf200, 0xf03c, 0xf10a,
        0xf10b, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf30e, 0xf702, 0xf30d,
        0xf01c, 0xf701, 0xf205, 0xf114, 0xf603, 0xf118, 0xf601, 0xf602, 0xf117, 0xf600, 0xf119,
        0xf115, 0xf116, 0xf11a, 0xf10c, 0xf10d, 0xf11b, 0xf11c, 0xf110, 0xf311, 0xf11d, 0xf200,
        0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200,
    ];
    const SHIFT_MAP: [u16; Self::NR_KEYS] = [
        0xf200, 0xf01b, 0xf021, 0xf040, 0xf023, 0xf024, 0xf025, 0xf05e, 0xf026, 0xf02a, 0xf028,
        0xf029, 0xf05f, 0xf02b, 0xf07f, 0xf009, 0xfb51, 0xfb57, 0xfb45, 0xfb52, 0xfb54, 0xfb59,
        0xfb55, 0xfb49, 0xfb4f, 0xfb50, 0xf07b, 0xf07d, 0xf201, 0xf702, 0xfb41, 0xfb53, 0xfb44,
        0xfb46, 0xfb47, 0xfb48, 0xfb4a, 0xfb4b, 0xfb4c, 0xf03a, 0xf022, 0xf07e, 0xf700, 0xf07c,
        0xfb5a, 0xfb58, 0xfb43, 0xfb56, 0xfb42, 0xfb4e, 0xfb4d, 0xf03c, 0xf03e, 0xf03f, 0xf700,
        0xf30c, 0xf703, 0xf020, 0xf207, 0xf10a, 0xf10b, 0xf10c, 0xf10d, 0xf10e, 0xf10f, 0xf110,
        0xf111, 0xf112, 0xf113, 0xf213, 0xf203, 0xf307, 0xf308, 0xf309, 0xf30b, 0xf304, 0xf305,
        0xf306, 0xf30a, 0xf301, 0xf302, 0xf303, 0xf300, 0xf310, 0xf206, 0xf200, 0xf03e, 0xf10a,
        0xf10b, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf30e, 0xf702, 0xf30d,
        0xf200, 0xf701, 0xf205, 0xf114, 0xf603, 0xf20b, 0xf601, 0xf602, 0xf117, 0xf600, 0xf20a,
        0xf115, 0xf116, 0xf11a, 0xf10c, 0xf10d, 0xf11b, 0xf11c, 0xf110, 0xf311, 0xf11d, 0xf200,
        0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200,
    ];
    const CTRL_MAP: [u16; Self::NR_KEYS] = [
        0xf200, 0xf200, 0xf200, 0xf000, 0xf01b, 0xf01c, 0xf01d, 0xf01e, 0xf01f, 0xf07f, 0xf200,
        0xf200, 0xf01f, 0xf200, 0xf008, 0xf200, 0xf011, 0xf017, 0xf005, 0xf012, 0xf014, 0xf019,
        0xf015, 0xf009, 0xf00f, 0xf010, 0xf01b, 0xf01d, 0xf201, 0xf702, 0xf001, 0xf013, 0xf004,
        0xf006, 0xf007, 0xf008, 0xf00a, 0xf00b, 0xf00c, 0xf200, 0xf007, 0xf000, 0xf700, 0xf01c,
        0xf01a, 0xf018, 0xf003, 0xf016, 0xf002, 0xf00e, 0xf00d, 0xf200, 0xf20e, 0xf07f, 0xf700,
        0xf30c, 0xf703, 0xf000, 0xf207, 0xf100, 0xf101, 0xf102, 0xf103, 0xf104, 0xf105, 0xf106,
        0xf107, 0xf108, 0xf109, 0xf208, 0xf204, 0xf307, 0xf308, 0xf309, 0xf30b, 0xf304, 0xf305,
        0xf306, 0xf30a, 0xf301, 0xf302, 0xf303, 0xf300, 0xf310, 0xf206, 0xf200, 0xf200, 0xf10a,
        0xf10b, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf30e, 0xf702, 0xf30d,
        0xf01c, 0xf701, 0xf205, 0xf114, 0xf603, 0xf118, 0xf601, 0xf602, 0xf117, 0xf600, 0xf119,
        0xf115, 0xf116, 0xf11a, 0xf10c, 0xf10d, 0xf11b, 0xf11c, 0xf110, 0xf311, 0xf11d, 0xf200,
        0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200, 0xf200,
    ];

    fn read() -> Self {
        Self(KEYBOARD_DATA_PORT.read())
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

    fn to_key(self, shift_key: bool, ctrl_key: bool, caps_lock: bool) -> Key {
        let code = self.0 & 0x7F;

        /* Handle special key */
        match code {
            0x01 => return Key::Escape,    /* Escape: 27 */
            0x0E => return Key::BackSpace, /* Backspace: 8 */
            0x1C => return Key::Enter,     /* Enter: 13 */
            0x47 => return Key::Home,      /* Home: 1 */
            0x48 => return Key::Up,        /* Up: 16 */
            0x4B => return Key::Left,      /* Left: 2 */
            0x4D => return Key::Right,     /* Right: 6 */
            0x4F => return Key::End,       /* End: 5 */
            0x50 => return Key::Down,      /* Down: 14 */
            0x53 => return Key::Delete,    /* Del: 4 */
            _ => (),
        }

        let mapped_code = if !caps_lock && !shift_key && !ctrl_key {
            Self::PLAIN_MAP[code as usize]
        } else if caps_lock || shift_key {
            Self::SHIFT_MAP[code as usize]
        } else if ctrl_key {
            Self::CTRL_MAP[code as usize]
        } else {
            log::debug!("unknown state/scancode: 0x{:X}", code);
            0x0020
        };

        let code = (mapped_code & 0xFF) as u8;
        match KeyType::from(((mapped_code & 0x0F00) >> 8) as u8) {
            KeyType::Latin | KeyType::Letter => {
                let char = char::from_u32(code as u32).unwrap();
                if ctrl_key {
                    Key::Ctrl(char)
                } else {
                    Key::Char(char)
                }
            }
            KeyType::Fn => Key::Fn(code + 1),
            KeyType::Unknown => Key::Null,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum KeyType {
    Latin,
    Fn,
    Letter,
    Unknown,
}

impl From<u8> for KeyType {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Latin,
            1 => Self::Fn,
            11 => Self::Letter,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct KeyStatus(u8);

impl KeyStatus {
    const STAT_OUTPUT_BUFFER_FULL: u8 = 0x01; /* Keyboard output buffer full */

    fn read() -> Self {
        Self(KEYBOARD_STATUS_PORT.read())
    }

    fn is_valid(&self) -> bool {
        self.0 != 0xFF
    }

    fn keyboard_buffer_is_full(&self) -> bool {
        self.0 & Self::STAT_OUTPUT_BUFFER_FULL == 0
    }
}

fn keyboard_getkey() -> Key {
    static CAPS_LOCK: AtomicBool = AtomicBool::new(false); /* CAPS LOCK state (0-off, 1-on) */
    static SHIFT_KEY: AtomicBool = AtomicBool::new(false); /* Shift next keypress */
    static CTRL_KEY: AtomicBool = AtomicBool::new(false);

    let code = KeyCode::read();
    let status = KeyStatus::read();

    if !code.is_valid() || !status.is_valid() {
        log::debug!("keyboard does not exist");
        return Key::Null;
    }

    if status.keyboard_buffer_is_full() {
        log::debug!("keyboard output buffer full");
    }

    /* Skip extension code */
    if code.is_extension() {
        return Key::Null;
    }

    // Ignore release, trigger on make, except for shift keys, where
    // we want to keep the shift state so long as the key is held down.
    if code.is_shift() {
        if code.is_pressed() {
            SHIFT_KEY.store(true, Ordering::Relaxed);
        } else {
            SHIFT_KEY.store(false, Ordering::Relaxed);
        }
        return Key::Null;
    }

    if code.is_ctrl() {
        if code.is_pressed() {
            CTRL_KEY.store(true, Ordering::Relaxed);
        } else {
            CTRL_KEY.store(false, Ordering::Relaxed);
        }
        return Key::Null;
    }

    if code.is_pressed() && code.is_caps_lock() {
        CAPS_LOCK.fetch_xor(true, Ordering::Relaxed);
        return Key::Null;
    }

    /* Ignore other release events */
    if code.is_released() {
        return Key::Null;
    }

    let shift_key = SHIFT_KEY.load(Ordering::Relaxed);
    let ctrl_key = CTRL_KEY.load(Ordering::Relaxed);
    let caps_lock = CAPS_LOCK.load(Ordering::Relaxed);
    code.to_key(shift_key, ctrl_key, caps_lock)
}
