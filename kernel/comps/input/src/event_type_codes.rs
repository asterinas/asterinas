// SPDX-License-Identifier: MPL-2.0

//! Event types and codes.
//!
//! This file is based on the USB HUT 1.12 specification
//! (see <http://www.usb.org/developers/hidpage>) and follows the
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

    /// Gets the event index as a `u16` value for this event type.
    ///
    /// # Panics
    ///
    /// This method will panic if there are multiple event bits set in `self`.
    pub const fn as_index(&self) -> u16 {
        // Check if exactly one bit is set
        let bits = self.bits();
        assert!(
            bits != 0 && (bits & (bits - 1)) == 0,
            "`EventTypes::as_index` expects exactly one event bit to be set"
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
            _ => unreachable!(),
        }
    }
}

/// Synchronization events.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynEvent {
    Report = 0x00,
    Config = 0x01,
    MtReport = 0x02,
    Dropped = 0x03,
}

/// Relative axes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelCode {
    X = 0x00,
    Y = 0x01,
    Z = 0x02,
    Rx = 0x03,
    Ry = 0x04,
    Rz = 0x05,
    HWheel = 0x06,
    Dial = 0x07,
    Wheel = 0x08,
    Misc = 0x09,
    Reserved = 0x0a,
    WheelHiRes = 0x0b,
    HWheelHiRes = 0x0c,
}
/// The maximum value for relative axes.
const REL_MAX: usize = 0x0f;
/// The number of relative axes.
const REL_COUNT: usize = REL_MAX + 1;

/// A set of [`RelCode`] represented as a bitmap.
#[derive(Debug, Clone)]
pub struct RelCodeSet(BitVec<u8>);

impl Default for RelCodeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl RelCodeSet {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self(BitVec::repeat(false, REL_COUNT))
    }

    /// Sets a relative code in the set.
    pub fn set(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        self.0.set(index, true);
    }

    /// Clears a relative code from the set.
    pub fn clear(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        self.0.set(index, false);
    }

    /// Checks if the set contains a relative code.
    pub fn contain(&self, rel_code: RelCode) -> bool {
        let index = rel_code as usize;
        self.0.get(index).map(|b| *b).unwrap()
    }

    /// Returns the bitmap as a byte slice.
    pub fn as_raw_slice(&self) -> &[u8] {
        self.0.as_raw_slice()
    }
}

/// A set of [`KeyCode`] represented as a bitmap.
#[derive(Debug, Clone)]
pub struct KeyCodeSet(BitVec<u8>);

impl Default for KeyCodeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCodeSet {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self(BitVec::repeat(false, KEY_COUNT))
    }

    /// Sets a key code in the set.
    pub fn set(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        self.0.set(index, true);
    }

    /// Clears a key code from the set.
    pub fn clear(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        self.0.set(index, false);
    }

    /// Checks if the set contains a key code.
    pub fn contain(&self, key_code: KeyCode) -> bool {
        let index = key_code as usize;
        self.0.get(index).map(|b| *b).unwrap()
    }

    /// Checks if the set contains any key codes in the `range`.
    ///
    /// # Panics
    ///
    /// This method will panic if the `range` contains invalid key codes.
    pub fn contain_any(&self, range: core::ops::Range<usize>) -> bool {
        assert!(range.is_empty() || range.end <= KEY_COUNT);
        range.into_iter().any(|i| *self.0.get(i).unwrap())
    }

    /// Returns the bitmap as a byte slice.
    pub fn as_raw_slice(&self) -> &[u8] {
        self.0.as_raw_slice()
    }
}

/// Common keyboard and mouse keys.
// TODO: Add more uncommon key codes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    // Reserved key.
    Reserved = 0,

    // Function keys.
    Esc = 1,
    F1 = 59,
    F2 = 60,
    F3 = 61,
    F4 = 62,
    F5 = 63,
    F6 = 64,
    F7 = 65,
    F8 = 66,
    F9 = 67,
    F10 = 68,
    F11 = 87,
    F12 = 88,

    // Number row.
    Num1 = 2,
    Num2 = 3,
    Num3 = 4,
    Num4 = 5,
    Num5 = 6,
    Num6 = 7,
    Num7 = 8,
    Num8 = 9,
    Num9 = 10,
    Num0 = 11,
    Minus = 12,
    Equal = 13,
    Backspace = 14,

    // First row (QWERTY).
    Tab = 15,
    Q = 16,
    W = 17,
    E = 18,
    R = 19,
    T = 20,
    Y = 21,
    U = 22,
    I = 23,
    O = 24,
    P = 25,
    LeftBrace = 26,  // [
    RightBrace = 27, // ]
    Backslash = 43,  // \

    // Second row (ASDF).
    CapsLock = 58,
    A = 30,
    S = 31,
    D = 32,
    F = 33,
    G = 34,
    H = 35,
    J = 36,
    K = 37,
    L = 38,
    Semicolon = 39,  // ;
    Apostrophe = 40, // '
    Enter = 28,

    // Third row (ZXCV).
    LeftShift = 42,
    Z = 44,
    X = 45,
    C = 46,
    V = 47,
    B = 48,
    N = 49,
    M = 50,
    Comma = 51,
    Dot = 52,
    Slash = 53, // /
    RightShift = 54,

    // Bottom row.
    LeftCtrl = 29,
    LeftAlt = 56,
    Space = 57,
    RightAlt = 100,
    RightCtrl = 97,

    // Special keys.
    Grave = 41,     // `
    LeftMeta = 125, // Windows/Cmd key
    RightMeta = 126,
    Menu = 139, // Context menu key

    // Arrow keys.
    Up = 103,
    Down = 108,
    Left = 105,
    Right = 106,

    // Navigation cluster.
    Home = 102,
    End = 107,
    PageUp = 104,
    PageDown = 109,
    Insert = 110,
    Delete = 111,

    // Common modifier states.
    NumLock = 69,
    ScrollLock = 70,

    // Numpad.
    Kp0 = 82,
    Kp1 = 79,
    Kp2 = 80,
    Kp3 = 81,
    Kp4 = 75,
    Kp5 = 76,
    Kp6 = 77,
    Kp7 = 71,
    Kp8 = 72,
    Kp9 = 73,
    KpDot = 83,
    KpPlus = 78,
    KpMinus = 74,
    KpAsterisk = 55, // *
    KpSlash = 98,    // /
    KpEnter = 96,

    // Common media keys.
    Mute = 113,
    VolumeDown = 114,
    VolumeUp = 115,

    // Misc keys.
    Power = 116,
    Pause = 119,

    // Starting code of button events.
    BtnMisc = 0x100,

    // Mouse buttons.
    BtnLeft = 0x110,
    BtnRight = 0x111,
    BtnMiddle = 0x112,
    BtnSide = 0x113,    // Mouse side button
    BtnExtra = 0x114,   // Mouse extra button
    BtnForward = 0x115, // Mouse forward button
    BtnBack = 0x116,    // Mouse back button
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
