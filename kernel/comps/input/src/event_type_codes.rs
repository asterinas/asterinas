// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;
use bitvec::prelude::*;

bitflags! {
    /// Input event types.
    pub struct EventTypes: u32 {
        /// Synchronization events
        const SYN = 1 << 0x00;
        /// Key press/release events
        const KEY = 1 << 0x01;
        /// Relative movement events (mouse, trackball, etc.)
        const REL = 1 << 0x02;
        /// Absolute position events (touchpad, tablet, etc.)
        const ABS = 1 << 0x03;
        /// Miscellaneous events
        const MSC = 1 << 0x04;
        /// Switch events
        const SW = 1 << 0x05;
        /// LED events
        const LED = 1 << 0x11;
        /// Sound events
        const SND = 1 << 0x12;
        /// Repeat events
        const REP = 1 << 0x14;
        /// Force feedback events
        const FF = 1 << 0x15;
        /// Power management events
        const PWR = 1 << 0x16;
        /// Force feedback status events
        const FF_STATUS = 1 << 0x17;
    }
}

impl Default for EventTypes {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTypes {
    /// Create a new empty set of event type flags
    pub const fn new() -> Self {
        Self::empty()
    }

    /// Get the raw u16 value for this event type
    pub const fn as_u16(&self) -> u16 {
        if self.contains(Self::SYN) {
            0x00
        } else if self.contains(Self::KEY) {
            0x01
        } else if self.contains(Self::REL) {
            0x02
        } else if self.contains(Self::ABS) {
            0x03
        } else if self.contains(Self::MSC) {
            0x04
        } else if self.contains(Self::SW) {
            0x05
        } else if self.contains(Self::LED) {
            0x11
        } else if self.contains(Self::SND) {
            0x12
        } else if self.contains(Self::REP) {
            0x14
        } else if self.contains(Self::FF) {
            0x15
        } else if self.contains(Self::PWR) {
            0x16
        } else if self.contains(Self::FF_STATUS) {
            0x17
        } else {
            0
        }
    }
}

/// Synchronization events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynEvent {
    SynReport = 0x00,
    SynConfig = 0x01,
    SynMtReport = 0x02,
    SynDropped = 0x03,
}

/// Relative axes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelEvent {
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
// Maximum value for relative axes
const REL_MAX: usize = 0x0f;
const REL_COUNT: usize = REL_MAX + 1;

#[derive(Debug, Clone)]
pub struct RelEventMap(BitVec<u8>);

impl Default for RelEventMap {
    fn default() -> Self {
        Self::new()
    }
}

impl RelEventMap {
    pub fn new() -> Self {
        // Initialize with all zeros, sized to hold all possible relative events
        Self(BitVec::repeat(false, REL_COUNT))
    }

    /// Set a relative event as supported
    pub fn set(&mut self, rel_event: RelEvent) {
        let index = rel_event as usize;
        if index < REL_COUNT {
            self.0.set(index, true);
        }
    }

    /// Clear a relative event
    pub fn clear(&mut self, rel_event: RelEvent) {
        let index = rel_event as usize;
        if index < REL_COUNT {
            self.0.set(index, false);
        }
    }

    /// Check if a relative event is supported
    pub fn contains(&self, rel_event: RelEvent) -> bool {
        let index = rel_event as usize;
        if index < REL_COUNT {
            self.0.get(index).map(|bit| *bit).unwrap_or(false)
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeyEventMap(BitVec<u8>);

impl Default for KeyEventMap {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyEventMap {
    pub fn new() -> Self {
        Self(BitVec::repeat(false, KEY_COUNT))
    }

    /// Set a key event as supported
    pub fn set(&mut self, key_event: KeyEvent) {
        let index = key_event as usize;
        if index < KEY_COUNT {
            self.0.set(index, true);
        }
    }

    /// Clear a key event
    pub fn clear(&mut self, key_event: KeyEvent) {
        let index = key_event as usize;
        if index < KEY_COUNT {
            self.0.set(index, false);
        }
    }

    /// Check if a key event is supported
    pub fn contains(&self, key_event: KeyEvent) -> bool {
        let index = key_event as usize;
        if index < KEY_COUNT {
            self.0.get(index).map(|bit| *bit).unwrap_or(false)
        } else {
            false
        }
    }
}

/// Common keyboard and mouse keys
// TODO: Add more uncommon key events.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    // Reserved key
    KeyReserved = 0,

    // Function keys
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

    // Number row
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

    // First row (QWERTY)
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

    // Second row (ASDF)
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

    // Third row (ZXCV)
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

    // Bottom row
    KeyLeftCtrl = 29,
    KeyLeftAlt = 56,
    KeySpace = 57,
    KeyRightAlt = 100,
    KeyRightCtrl = 97,

    // Special keys
    KeyGrave = 41,     // `
    KeyLeftMeta = 125, // Windows/Cmd key
    KeyRightMeta = 126,
    KeyMenu = 139, // Context menu key

    // Arrow keys
    KeyUp = 103,
    KeyDown = 108,
    KeyLeft = 105,
    KeyRight = 106,

    // Navigation cluster
    KeyHome = 102,
    KeyEnd = 107,
    KeyPageUp = 104,
    KeyPageDown = 109,
    KeyInsert = 110,
    KeyDelete = 111,

    // Common modifier states
    KeyNumLock = 69,
    KeyScrollLock = 70,

    // Numpad
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

    // Common media keys
    KeyMute = 113,
    KeyVolumeDown = 114,
    KeyVolumeUp = 115,

    // Mouse buttons
    BtnLeft = 0x110,
    BtnRight = 0x111,
    BtnMiddle = 0x112,
    BtnSide = 0x113,    // Mouse side button
    BtnExtra = 0x114,   // Mouse extra button
    BtnForward = 0x115, // Mouse forward button
    BtnBack = 0x116,    // Mouse back button
}
// Maximum value for key events
const KEY_MAX: usize = 0x120;
const KEY_COUNT: usize = KEY_MAX + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Released,
    Pressed,
}
