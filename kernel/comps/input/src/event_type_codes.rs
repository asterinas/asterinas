// SPDX-License-Identifier: MPL-2.0

//! Event types and codes.
//!
//! This file is based on the USB HUT 1.12 specification
//! (see http://www.usb.org/developers/hidpage) and follows the
//! Linux input event codes standard for maximum compatibility.
//!
//! The USB HUT standard ensures consistent event type and value
//! definitions across different input devices and platforms.

use bitflags::bitflags;
use bitvec::prelude::*;

bitflags! {
    /// Input event types.
    pub struct EventTypes: u32 {
        /// Synchronization events.
        const SYN = 1 << 0x00;
        /// Key press/release events.
        const KEY = 1 << 0x01;
        /// Relative movement events. (mouse, trackball, etc.)
        const REL = 1 << 0x02;
        /// Absolute position events. (touchpad, tablet, etc.)
        const ABS = 1 << 0x03;
        /// Miscellaneous events.
        const MSC = 1 << 0x04;
        /// Switch events.
        const SW = 1 << 0x05;
        /// LED events.
        const LED = 1 << 0x11;
        /// Sound events.
        const SND = 1 << 0x12;
        /// Repeat events.
        const REP = 1 << 0x14;
        /// Force feedback events.
        const FF = 1 << 0x15;
        /// Power management events.
        const PWR = 1 << 0x16;
        /// Force feedback status events.
        const FF_STATUS = 1 << 0x17;
    }
}

impl Default for EventTypes {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTypes {
    /// Creates a new empty set of event type flags.
    pub const fn new() -> Self {
        Self::empty()
    }

    /// Gets the raw u16 value for this event type.
    pub const fn as_u16(&self) -> u16 {
        let bits = self.bits();
        // Check if exactly one bit is set
        assert!(
            bits != 0 && (bits & (bits - 1)) == 0,
            "EventTypes::as_u16() expects exactly one flag to be set."
        );
        match *self {
            Self::SYN => 0x00,
            Self::KEY => 0x01,
            Self::REL => 0x02,
            Self::ABS => 0x03,
            Self::MSC => 0x04,
            Self::SW => 0x05,
            Self::LED => 0x11,
            Self::SND => 0x12,
            Self::REP => 0x14,
            Self::FF => 0x15,
            Self::PWR => 0x16,
            Self::FF_STATUS => 0x17,
            _ => 0,
        }
    }
}

/// Synchronization events.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynEvent {
    SynReport = 0x00,
    SynConfig = 0x01,
    SynMtReport = 0x02,
    SynDropped = 0x03,
}

/// Relative axes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelCode {
    RelX = 0x00,
    RelY = 0x01,
    RelZ = 0x02,
    RelRx = 0x03,
    RelRy = 0x04,
    RelRz = 0x05,
    RelHWheel = 0x06,
    RelDial = 0x07,
    RelWheel = 0x08,
    RelMisc = 0x09,
    RelReserved = 0x0a,
    RelWheelHiRes = 0x0b,
    RelHWheelHiRes = 0x0c,
}
/// The maximum value for relative axes.
const REL_MAX: usize = 0x0f;
/// The number of relative axes.
const REL_COUNT: usize = REL_MAX + 1;

#[derive(Debug, Clone)]
pub struct RelCodeMap(BitVec<u8>);

impl Default for RelCodeMap {
    fn default() -> Self {
        Self::new()
    }
}

impl RelCodeMap {
    pub fn new() -> Self {
        // Initialize with all zeros, sized to hold all possible relative codes.
        Self(BitVec::repeat(false, REL_COUNT))
    }

    /// Sets a relative code as supported.
    pub fn set(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        if index < REL_COUNT {
            self.0.set(index, true);
        }
    }

    /// Clears a relative code.
    pub fn clear(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        if index < REL_COUNT {
            self.0.set(index, false);
        }
    }

    /// Checks if a relative code is supported.
    pub fn contain(&self, rel_code: RelCode) -> bool {
        let index = rel_code as usize;
        if index < REL_COUNT {
            self.0.get(index).map(|bit| *bit).unwrap_or(false)
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeyCodeMap(BitVec<u8>);

impl Default for KeyCodeMap {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCodeMap {
    pub fn new() -> Self {
        Self(BitVec::repeat(false, KEY_COUNT))
    }

    /// Sets a key code as supported.
    pub fn set(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        if index < KEY_COUNT {
            self.0.set(index, true);
        }
    }

    /// Clears a key code.
    pub fn clear(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        if index < KEY_COUNT {
            self.0.set(index, false);
        }
    }

    /// Checks if a key code is supported.
    pub fn contain(&self, key_code: KeyCode) -> bool {
        let index = key_code as usize;
        if index < KEY_COUNT {
            self.0.get(index).map(|bit| *bit).unwrap_or(false)
        } else {
            false
        }
    }

    /// Checks if any key bit is set in the range [start, end).
    pub fn contain_any_in_range(&self, start: usize, end: usize) -> bool {
        let len = self.0.len();
        let start = start.min(len);
        let end = end.min(len);
        if start >= end {
            return false;
        }
        for i in start..end {
            if self.0.get(i).map(|b| *b).unwrap_or(false) {
                return true;
            }
        }
        false
    }
}

/// Common keyboard and mouse keys.
// TODO: Add more uncommon key codes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    // Reserved key.
    KeyReserved = 0,

    // Function keys.
    KeyEsc = 1,
    KeyF1 = 59,
    KeyF2 = 60,
    KeyF3 = 61,
    KeyF4 = 62,
    KeyF5 = 63,
    KeyF6 = 64,
    KeyF7 = 65,
    KeyF8 = 66,
    KeyF9 = 67,
    KeyF10 = 68,
    KeyF11 = 87,
    KeyF12 = 88,

    // Number row.
    Key1 = 2,
    Key2 = 3,
    Key3 = 4,
    Key4 = 5,
    Key5 = 6,
    Key6 = 7,
    Key7 = 8,
    Key8 = 9,
    Key9 = 10,
    Key0 = 11,
    KeyMinus = 12,
    KeyEqual = 13,
    KeyBackspace = 14,

    // First row (QWERTY).
    KeyTab = 15,
    KeyQ = 16,
    KeyW = 17,
    KeyE = 18,
    KeyR = 19,
    KeyT = 20,
    KeyY = 21,
    KeyU = 22,
    KeyI = 23,
    KeyO = 24,
    KeyP = 25,
    KeyLeftBrace = 26,  // [
    KeyRightBrace = 27, // ]
    KeyBackslash = 43,  // \

    // Second row (ASDF).
    KeyCapsLock = 58,
    KeyA = 30,
    KeyS = 31,
    KeyD = 32,
    KeyF = 33,
    KeyG = 34,
    KeyH = 35,
    KeyJ = 36,
    KeyK = 37,
    KeyL = 38,
    KeySemicolon = 39,  // ;
    KeyApostrophe = 40, // '
    KeyEnter = 28,

    // Third row (ZXCV).
    KeyLeftShift = 42,
    KeyZ = 44,
    KeyX = 45,
    KeyC = 46,
    KeyV = 47,
    KeyB = 48,
    KeyN = 49,
    KeyM = 50,
    KeyComma = 51,
    KeyDot = 52,
    KeySlash = 53, // /
    KeyRightShift = 54,

    // Bottom row.
    KeyLeftCtrl = 29,
    KeyLeftAlt = 56,
    KeySpace = 57,
    KeyRightAlt = 100,
    KeyRightCtrl = 97,

    // Special keys.
    KeyGrave = 41,     // `
    KeyLeftMeta = 125, // Windows/Cmd key
    KeyRightMeta = 126,
    KeyMenu = 139, // Context menu key

    // Arrow keys.
    KeyUp = 103,
    KeyDown = 108,
    KeyLeft = 105,
    KeyRight = 106,

    // Navigation cluster.
    KeyHome = 102,
    KeyEnd = 107,
    KeyPageUp = 104,
    KeyPageDown = 109,
    KeyInsert = 110,
    KeyDelete = 111,

    // Common modifier states.
    KeyNumLock = 69,
    KeyScrollLock = 70,

    // Numpad.
    KeyKp0 = 82,
    KeyKp1 = 79,
    KeyKp2 = 80,
    KeyKp3 = 81,
    KeyKp4 = 75,
    KeyKp5 = 76,
    KeyKp6 = 77,
    KeyKp7 = 71,
    KeyKp8 = 72,
    KeyKp9 = 73,
    KeyKpDot = 83,
    KeyKpPlus = 78,
    KeyKpMinus = 74,
    KeyKpAsterisk = 55, // *
    KeyKpSlash = 98,    // /
    KeyKpEnter = 96,

    // Common media keys.
    KeyMute = 113,
    KeyVolumeDown = 114,
    KeyVolumeUp = 115,

    // Starting code of button events.
    KeyBtnMisc = 0x100,

    // Mouse buttons.
    KeyBtnLeft = 0x110,
    KeyBtnRight = 0x111,
    KeyBtnMiddle = 0x112,
    KeyBtnSide = 0x113,    // Mouse side button
    KeyBtnExtra = 0x114,   // Mouse extra button
    KeyBtnForward = 0x115, // Mouse forward button
    KeyBtnBack = 0x116,    // Mouse back button
}

/// The maximum value for key codes.
const KEY_MAX: usize = 0x120;
/// The number of key codes.
const KEY_COUNT: usize = KEY_MAX + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Released,
    Pressed,
}
