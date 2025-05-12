// SPDX-License-Identifier: MPL-2.0
/// Event types
use int_to_c_enum::TryFromInt;

/// Event types
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    EvSyn = 0x00,        // Synchronization events
    EvKey = 0x01,        // Key press/release events
    EvRel = 0x02,        // Relative movement events
    EvAbs = 0x03,        // Absolute position events
    EvMsc = 0x04,        // Miscellaneous events
    EvSw = 0x05,         // Switch events
    EvLed = 0x11,        // LED events
    EvSnd = 0x12,        // Sound events
    EvRep = 0x14,        // Repeat events
    EvFf = 0x15,         // Force feedback events
    EvPwr = 0x16,        // Power management events
    EvFfStatus = 0x17,   // Force feedback status events
    EvMax = 0x1f,        // Maximum value for event types
}

/// Synchronization events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynEvent {
    SynReport = 0x00,
    SynConfig = 0x01,
    SynMtReport = 0x02,
    SynDropped = 0x03,
    SynMax = 0x0f,
}

/// Relative axes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelAxis {
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
    RelMax = 0x0f,
}

/// Absolute axes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsAxis {
    AbsX = 0x00,
    AbsY = 0x01,
    AbsZ = 0x02,
    AbsRx = 0x03,
    AbsRy = 0x04,
    AbsRz = 0x05,
    AbsThrottle = 0x06,
    AbsRudder = 0x07,
    AbsWheel = 0x08,
    AbsGas = 0x09,
    AbsBrake = 0x0a,
    AbsHat0X = 0x10,
    AbsHat0Y = 0x11,
    AbsHat1X = 0x12,
    AbsHat1Y = 0x13,
    AbsHat2X = 0x14,
    AbsHat2Y = 0x15,
    AbsHat3X = 0x16,
    AbsHat3Y = 0x17,
    AbsPressure = 0x18,
    AbsDistance = 0x19,
    AbsTiltX = 0x1a,
    AbsTiltY = 0x1b,
    AbsToolWidth = 0x1c,
    AbsVolume = 0x20,
    AbsProfile = 0x21,
    AbsMisc = 0x28,
    AbsReserved = 0x2e,
    AbsMtSlot = 0x2f,
    AbsMtTouchMajor = 0x30,
    AbsMtTouchMinor = 0x31,
    AbsMtWidthMajor = 0x32,
    AbsMtWidthMinor = 0x33,
    AbsMtOrientation = 0x34,
    AbsMtPositionX = 0x35,
    AbsMtPositionY = 0x36,
    AbsMtToolType = 0x37,
    AbsMtBlobId = 0x38,
    AbsMtTrackingId = 0x39,
    AbsMtPressure = 0x3a,
    AbsMtDistance = 0x3b,
    AbsMtToolX = 0x3c,
    AbsMtToolY = 0x3d,
    AbsMax = 0x3f,
}

/// Switch events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchEvent {
    SwLid = 0x00,
    SwTabletMode = 0x01,
    SwHeadphoneInsert = 0x02,
    SwRfkillAll = 0x03,
    SwMicrophoneInsert = 0x04,
    SwDock = 0x05,
    SwLineoutInsert = 0x06,
    SwJackPhysicalInsert = 0x07,
    SwVideooutInsert = 0x08,
    SwCameraLensCover = 0x09,
    SwKeypadSlide = 0x0a,
    SwFrontProximity = 0x0b,
    SwRotateLock = 0x0c,
    SwLineinInsert = 0x0d,
    SwMuteDevice = 0x0e,
    SwPenInserted = 0x0f,
    SwMachineCover = 0x10,
}

/// Miscellaneous events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiscEvent {
    MscSerial = 0x00,
    MscPulseLed = 0x01,
    MscGesture = 0x02,
    MscRaw = 0x03,
    MscScan = 0x04,
    MscTimestamp = 0x05,
}

/// LED events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedEvent {
    LedNuml = 0x00,
    LedCapsl = 0x01,
    LedScrolll = 0x02,
    LedCompose = 0x03,
    LedKana = 0x04,
    LedSleep = 0x05,
    LedSuspend = 0x06,
    LedMute = 0x07,
    LedMisc = 0x08,
    LedMail = 0x09,
    LedCharging = 0x0a,
}

/// Sound events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEvent {
    SndClick = 0x00,
    SndBell = 0x01,
    SndTone = 0x02,
}

/// Mouse key events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKeyEvent {
    MouseLeft = 0x110,
    MouseRight = 0x111,
    MouseMiddle = 0x112,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, TryFromInt)]
#[repr(u16)]
pub enum Key {
    Reserved = 0,
    ESC = 1,
    One = 2,
    Two = 3,
    Three = 4,
    Four = 5,
    Five = 6,
    Six = 7,
    Seven = 8,
    Eight = 9,
    Nine = 10,
    Zero = 11,
    Minus = 12,
    Equal = 13,
    BackSpace = 14,
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
    /// Symbol: [
    LeftBrace = 26,
    /// Symbol: ]
    RightBrace = 27,
    Enter = 28,
    LeftCtrl = 29,
    A = 30,
    S = 31,
    D = 32,
    F = 33,
    G = 34,
    H = 35,
    J = 36,
    K = 37,
    L = 38,
    /// Symbol: ;
    SemiColon = 39,
    /// Symbol: '
    Apostrophe = 40,
    /// Symbol: `
    Grave = 41,
    LeftShift = 42,
    /// Symbol: \
    BackSlash = 43,
    Z = 44,
    X = 45,
    C = 46,
    V = 47,
    B = 48,
    N = 49,
    M = 50,
    Comma = 51,
    Dot = 52,
    // Symbol: /
    Slash = 53,
    RightShift = 54,
    /// Keypad asterisk, Symbol: *
    KpAsterisk = 55,
    LeftAlt = 56,
    Space = 57,
    Capslock = 58,
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
    NumLock = 69,
    ScrollLock = 70,
    Kp7 = 71,
    Kp8 = 72,
    Kp9 = 73,
    KpMinus = 74,
    Kp4 = 75,
    Kp5 = 76,
    Kp6 = 77,
    KpPlus = 78,
    Kp1 = 79,
    Kp2 = 80,
    Kp3 = 81,
    Kp0 = 82,
    KpDot = 83,

    F11 = 87,
    F12 = 88,

    KpEnter = 96,
    RightCtrl = 97,
    KpSlash = 98,

    RightAlt = 100,
    LineFeed = 101,
    Home = 102,
    Up = 103,
    PageUp = 104,
    Left = 105,
    Right = 106,
    End = 107,
    Down = 108,
    PageDown = 109,
    Insert = 110,
    Delete = 111,

    LeftMeta = 125,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Pressed,
    Released,
}