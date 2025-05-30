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
}
// Maximum value for event types
const EV_MAX: usize = 0x1f;
pub const EV_COUNT: usize = EV_MAX + 1;

impl TryFrom<u8> for EventType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(EventType::EvSyn),
            0x01 => Ok(EventType::EvKey),
            0x02 => Ok(EventType::EvRel),
            0x03 => Ok(EventType::EvAbs),
            0x04 => Ok(EventType::EvMsc),
            0x05 => Ok(EventType::EvSw),
            0x11 => Ok(EventType::EvLed),
            0x12 => Ok(EventType::EvSnd),
            0x14 => Ok(EventType::EvRep),
            0x15 => Ok(EventType::EvFf),
            0x16 => Ok(EventType::EvPwr),
            0x17 => Ok(EventType::EvFfStatus),
            _ => Err(()),
        }
    }
}

/// Device properties
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    Pointer = 0x00,         // Needs a pointer
    Direct = 0x01,          // Direct input devices
    ButtenPad = 0x02,       // Has button(s) under pad
    SemiMT = 0x03,          // Touch rectangle only
    TopButtonPad = 0x04,    // Softbuttons at top of pad
    PointingStick = 0x05,   // Is a pointing stick
    Accelerometer = 0x06,   // Has accelerometer       
}
// Maximum value for prop types
const PROP_MAX: usize = 0x1f;
pub const PROP_COUNT: usize = PROP_MAX + 1;

/// Synchronization events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynEvent {
    SynReport = 0x00,
    SynConfig = 0x01,
    SynMtReport = 0x02,
    SynDropped = 0x03,
}
// Maximum value for synchronization event types
const SYN_MAX: usize = 0x0f;
pub const SYN_COUNT: usize = SYN_MAX + 1;

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
pub const REL_COUNT: usize = REL_MAX + 1;

/// Absolute axes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsEvent {
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
}
// Maximum value for absolute axes
const ABS_MAX: usize = 0x3f;
pub const ABS_COUNT: usize = ABS_MAX + 1;

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
// Maximum value for switch events
const SW_MAX: usize = 0x10;
pub const SW_COUNT: usize = SW_MAX + 1;

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
    MscMax = 0x07,
}
// Maximum value for miscellaneous events
const MSC_MAX: usize = 0x07;
pub const MSC_COUNT: usize = MSC_MAX + 1;

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
// Maximum value for LED events
const LED_MAX: usize = 0x0f;
pub const LED_COUNT: usize = LED_MAX + 1;

/// Sound events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEvent {
    SndClick = 0x00,
    SndBell = 0x01,
    SndTone = 0x02,
}
// Maximum value for Sound events
const SND_MAX: usize = 0x07;
pub const SND_COUNT: usize = SND_MAX + 1;

/// Mouse key events
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKeyEvent {
    MouseLeft = 0x110,
    MouseRight = 0x111,
    MouseMiddle = 0x112,
}

// #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, TryFromInt)]
// #[repr(u16)]
// pub enum Key {
//     Reserved = 0,
//     ESC = 1,
//     One = 2,
//     Two = 3,
//     Three = 4,
//     Four = 5,
//     Five = 6,
//     Six = 7,
//     Seven = 8,
//     Eight = 9,
//     Nine = 10,
//     Zero = 11,
//     Minus = 12,
//     Equal = 13,
//     BackSpace = 14,
//     Tab = 15,
//     Q = 16,
//     W = 17,
//     E = 18,
//     R = 19,
//     T = 20,
//     Y = 21,
//     U = 22,
//     I = 23,
//     O = 24,
//     P = 25,
//     /// Symbol: [
//     LeftBrace = 26,
//     /// Symbol: ]
//     RightBrace = 27,
//     Enter = 28,
//     LeftCtrl = 29,
//     A = 30,
//     S = 31,
//     D = 32,
//     F = 33,
//     G = 34,
//     H = 35,
//     J = 36,
//     K = 37,
//     L = 38,
//     /// Symbol: ;
//     SemiColon = 39,
//     /// Symbol: '
//     Apostrophe = 40,
//     /// Symbol: `
//     Grave = 41,
//     LeftShift = 42,
//     /// Symbol: \
//     BackSlash = 43,
//     Z = 44,
//     X = 45,
//     C = 46,
//     V = 47,
//     B = 48,
//     N = 49,
//     M = 50,
//     Comma = 51,
//     Dot = 52,
//     // Symbol: /
//     Slash = 53,
//     RightShift = 54,
//     /// Keypad asterisk, Symbol: *
//     KpAsterisk = 55,
//     LeftAlt = 56,
//     Space = 57,
//     Capslock = 58,
//     F1 = 59,
//     F2 = 60,
//     F3 = 61,
//     F4 = 62,
//     F5 = 63,
//     F6 = 64,
//     F7 = 65,
//     F8 = 66,
//     F9 = 67,
//     F10 = 68,
//     NumLock = 69,
//     ScrollLock = 70,
//     Kp7 = 71,
//     Kp8 = 72,
//     Kp9 = 73,
//     KpMinus = 74,
//     Kp4 = 75,
//     Kp5 = 76,
//     Kp6 = 77,
//     KpPlus = 78,
//     Kp1 = 79,
//     Kp2 = 80,
//     Kp3 = 81,
//     Kp0 = 82,
//     KpDot = 83,

//     F11 = 87,
//     F12 = 88,

//     KpEnter = 96,
//     RightCtrl = 97,
//     KpSlash = 98,

//     RightAlt = 100,
//     LineFeed = 101,
//     Home = 102,
//     Up = 103,
//     PageUp = 104,
//     Left = 105,
//     Right = 106,
//     End = 107,
//     Down = 108,
//     PageDown = 109,
//     Insert = 110,
//     Delete = 111,

//     LeftMeta = 125,
// }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Released,
    Pressed,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    KeyReserved = 0,
    KeyEsc = 1,
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
    /// Symbol: [
    KeyLeftBrace = 26,
    /// Symbol: ]
    KeyRightBrace = 27,
    KeyEnter = 28,
    KeyLeftCtrl = 29,
    KeyA = 30,
    KeyS = 31,
    KeyD = 32,
    KeyF = 33,
    KeyG = 34,
    KeyH = 35,
    KeyJ = 36,
    KeyK = 37,
    KeyL = 38,
    /// Symbol: ;
    KeySemicolon = 39,
    /// Symbol: '
    KeyApostrophe = 40,
    /// Symbol: `
    KeyGrave = 41,
    KeyLeftShift = 42,
    /// Symbol: \
    KeyBackslash = 43,
    KeyZ = 44,
    KeyX = 45,
    KeyC = 46,
    KeyV = 47,
    KeyB = 48,
    KeyN = 49,
    KeyM = 50,
    KeyComma = 51,
    KeyDot = 52,
    // Symbol: /
    KeySlash = 53,
    KeyRightShift = 54,
    /// Keypad asterisk, Symbol: *
    KeyKpAsterisk = 55,
    KeyLeftAlt = 56,
    KeySpace = 57,
    KeyCapsLock = 58,
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
    KeyNumLock = 69,
    KeyScrollLock = 70,
    KeyKp7 = 71,
    KeyKp8 = 72,
    KeyKp9 = 73,
    KeyKpMinus = 74,
    KeyKp4 = 75,
    KeyKp5 = 76,
    KeyKp6 = 77,
    KeyKpPlus = 78,
    KeyKp1 = 79,
    KeyKp2 = 80,
    KeyKp3 = 81,
    KeyKp0 = 82,
    KeyKpDot = 83,

    KeyZenkakuHankaku = 85,
    Key102nd = 86,
    KeyF11 = 87,
    KeyF12 = 88,
    KeyRo = 89,
    KeyKatakana = 90,
    KeyHiragana = 91,
    KeyHenkan = 92,
    KeyKatakanaHiragana = 93,
    KeyMuhenkan = 94,
    KeyKpJpComma = 95,
    KeyKpEnter = 96,
    KeyRightCtrl = 97,
    KeyKpSlash = 98,
    KeySysRq = 99,
    KeyRightAlt = 100,
    KeyLineFeed = 101,
    KeyHome = 102,
    KeyUp = 103,
    KeyPageUp = 104,
    KeyLeft = 105,
    KeyRight = 106,
    KeyEnd = 107,
    KeyDown = 108,
    KeyPageDown = 109,
    KeyInsert = 110,
    KeyDelete = 111,
    KeyMacro = 112,
    KeyMute = 113,
    KeyVolumeDown = 114,
    KeyVolumeUp = 115,
    KeyPower = 116,
    KeyKpEqual = 117,
    KeyKpPlusMinus = 118,
    KeyPause = 119,
    KeyScale = 120,
    KeyKpComma = 121,
    KeyHangeul = 122,
    KeyHanja = 123,
    KeyYen = 124,
    KeyLeftMeta = 125,
    KeyRightMeta = 126,
    KeyCompose = 127,

    KeyStop = 128,
    KeyAgain = 129,
    KeyProps = 130,
    KeyUndo = 131,
    KeyFront = 132,
    KeyCopy = 133,
    KeyOpen = 134,
    KeyPaste = 135,
    KeyFind = 136,
    KeyCut = 137,
    KeyHelp = 138,
    KeyMenu = 139,
    KeyCalc = 140,
    KeySetup = 141,
    KeySleep = 142,
    KeyWakeup = 143,
    KeyFile = 144,
    KeySendFile = 145,
    KeyDeleteFile = 146,
    KeyXfer = 147,
    KeyProg1 = 148,
    KeyProg2 = 149,
    KeyWww = 150,
    KeyMsdos = 151,
    KeyCoffee = 152,         // Also known as KeyScreenLock
    KeyRotateDisplay = 153,  // Also known as KeyDirection
    KeyCycleWindows = 154,
    KeyMail = 155,
    KeyBookmarks = 156,
    KeyComputer = 157,
    KeyBack = 158,
    KeyForward = 159,
    KeyCloseCd = 160,
    KeyEjectCd = 161,
    KeyEjectCloseCd = 162,
    KeyNextSong = 163,
    KeyPlayPause = 164,
    KeyPreviousSong = 165,
    KeyStopCd = 166,
    KeyRecord = 167,
    KeyRewind = 168,
    KeyPhone = 169,
    KeyIso = 170,
    KeyConfig = 171,
    KeyHomepage = 172,
    KeyRefresh = 173,
    KeyExit = 174,
    KeyMove = 175,
    KeyEdit = 176,
    KeyScrollUp = 177,
    KeyScrollDown = 178,
    KeyKpLeftParen = 179,
    KeyKpRightParen = 180,
    KeyNew = 181,
    KeyRedo = 182,

    KeyF13 = 183,
    KeyF14 = 184,
    KeyF15 = 185,
    KeyF16 = 186,
    KeyF17 = 187,
    KeyF18 = 188,
    KeyF19 = 189,
    KeyF20 = 190,
    KeyF21 = 191,
    KeyF22 = 192,
    KeyF23 = 193,
    KeyF24 = 194,

    KeyPlayCd = 200,
    KeyPauseCd = 201,
    KeyProg3 = 202,
    KeyProg4 = 203,
    KeyAllApplications = 204, // Also known as KeyDashboard
    KeySuspend = 205,
    KeyClose = 206,
    KeyPlay = 207,
    KeyFastForward = 208,
    KeyBassBoost = 209,
    KeyPrint = 210,
    KeyHp = 211,
    KeyCamera = 212,
    KeySound = 213,
    KeyQuestion = 214,
    KeyEmail = 215,
    KeyChat = 216,
    KeySearch = 217,
    KeyConnect = 218,
    KeyFinance = 219,
    KeySport = 220,
    KeyShop = 221,
    KeyAltErase = 222,
    KeyCancel = 223,
    KeyBrightnessDown = 224,
    KeyBrightnessUp = 225,
    KeyMedia = 226,

    KeySwitchVideoMode = 227,
    KeyKbdIllumToggle = 228,
    KeyKbdIllumDown = 229,
    KeyKbdIllumUp = 230,

    KeySend = 231,
    KeyReply = 232,
    KeyForwardMail = 233,
    KeySave = 234,
    KeyDocuments = 235,

    KeyBattery = 236,

    KeyBluetooth = 237,
    KeyWlan = 238,
    KeyUwb = 239,

    KeyUnknown = 240,
    KeyVideoNext = 241,
    KeyVideoPrev = 242,
    KeyBrightnessCycle = 243,
    KeyBrightnessAuto = 244, // Also known as KeyBrightnessZero
    KeyDisplayOff = 245,
    KeyWwan = 246,           // Also known as KeyWimax
    KeyRfKill = 247,
    KeyMicMute = 248,

    Btn0 = 0x100,
    Btn1 = 0x101,
    Btn2 = 0x102,
    Btn3 = 0x103,
    Btn4 = 0x104,
    Btn5 = 0x105,
    Btn6 = 0x106,
    Btn7 = 0x107,
    Btn8 = 0x108,
    Btn9 = 0x109,

    BtnLeft = 0x110,
    BtnRight = 0x111,
    BtnMiddle = 0x112,
    BtnSide = 0x113,
    BtnExtra = 0x114,
    BtnForward = 0x115,
    BtnBack = 0x116,
    BtnTask = 0x117,

    BtnTrigger = 0x120,
    BtnThumb = 0x121,
    BtnThumb2 = 0x122,
    BtnTop = 0x123,
    BtnTop2 = 0x124,
    BtnPinkie = 0x125,
    BtnBase = 0x126,
    BtnBase2 = 0x127,
    BtnBase3 = 0x128,
    BtnBase4 = 0x129,
    BtnBase5 = 0x12a,
    BtnBase6 = 0x12b,
    BtnDead = 0x12f,

    BtnSouth = 0x130, // Also BtnA
    BtnEast = 0x131,  // Also BtnB
    BtnC = 0x132,
    BtnNorth = 0x133, // Also BtnX
    BtnWest = 0x134,  // Also BtnY
    BtnZ = 0x135,
    BtnTl = 0x136,
    BtnTr = 0x137,
    BtnTl2 = 0x138,
    BtnTr2 = 0x139,
    BtnSelect = 0x13a,
    BtnStart = 0x13b,
    BtnMode = 0x13c,
    BtnThumbL = 0x13d,
    BtnThumbR = 0x13e,

    BtnToolPen = 0x140,
    BtnToolRubber = 0x141,
    BtnToolBrush = 0x142,
    BtnToolPencil = 0x143,
    BtnToolAirbrush = 0x144,
    BtnToolFinger = 0x145,
    BtnToolMouse = 0x146,
    BtnToolLens = 0x147,
    BtnToolQuintTap = 0x148,
    BtnStylus3 = 0x149,
    BtnTouch = 0x14a,
    BtnStylus = 0x14b,
    BtnStylus2 = 0x14c,
    BtnToolDoubleTap = 0x14d,
    BtnToolTripleTap = 0x14e,
    BtnToolQuadTap = 0x14f,

    BtnGearDown = 0x150,
    BtnGearUp = 0x151,

    KeyOk = 0x160,
    KeySelect = 0x161,
    KeyGoto = 0x162,
    KeyClear = 0x163,
    KeyPower2 = 0x164,
    KeyOption = 0x165,
    KeyInfo = 0x166,
    KeyTime = 0x167,
    KeyVendor = 0x168,
    KeyArchive = 0x169,
    KeyProgram = 0x16a,
    KeyChannel = 0x16b,
    KeyFavorites = 0x16c,
    KeyEpg = 0x16d,
    KeyPvr = 0x16e,
    KeyMhp = 0x16f,
    KeyLanguage = 0x170,
    KeyTitle = 0x171,
    KeySubtitle = 0x172,
    KeyAngle = 0x173,
    KeyFullScreen = 0x174,
    KeyMode = 0x175,
    KeyKeyboard = 0x176,
    KeyAspectRatio = 0x177,
    KeyPc = 0x178,
    KeyTv = 0x179,
    KeyTv2 = 0x17a,
    KeyVcr = 0x17b,
    KeyVcr2 = 0x17c,
    KeySat = 0x17d,
    KeySat2 = 0x17e,
    KeyCd = 0x17f,
    KeyTape = 0x180,
    KeyRadio = 0x181,
    KeyTuner = 0x182,
    KeyPlayer = 0x183,
    KeyText = 0x184,
    KeyDvd = 0x185,
    KeyAux = 0x186,
    KeyMp3 = 0x187,
    KeyAudio = 0x188,
    KeyVideo = 0x189,
    KeyDirectory = 0x18a,
    KeyList = 0x18b,
    KeyMemo = 0x18c,
    KeyCalendar = 0x18d,
    KeyRed = 0x18e,
    KeyGreen = 0x18f,
    KeyYellow = 0x190,
    KeyBlue = 0x191,
    KeyChannelUp = 0x192,
    KeyChannelDown = 0x193,
    KeyFirst = 0x194,
    KeyLast = 0x195,
    KeyAb = 0x196,
    KeyNext = 0x197,
    KeyRestart = 0x198,
    KeySlow = 0x199,
    KeyShuffle = 0x19a,
    KeyBreak = 0x19b,
    KeyPrevious = 0x19c,
    KeyDigits = 0x19d,
    KeyTeen = 0x19e,
    KeyTwen = 0x19f,
    KeyVideoPhone = 0x1a0,
    KeyGames = 0x1a1,
    KeyZoomIn = 0x1a2,
    KeyZoomOut = 0x1a3,
    KeyZoomReset = 0x1a4,
    KeyWordProcessor = 0x1a5,
    KeyEditor = 0x1a6,
    KeySpreadsheet = 0x1a7,
    KeyGraphicsEditor = 0x1a8,
    KeyPresentation = 0x1a9,
    KeyDatabase = 0x1aa,
    KeyNews = 0x1ab,
    KeyVoicemail = 0x1ac,
    KeyAddressBook = 0x1ad,
    KeyMessenger = 0x1ae,
    KeyDisplayToggle = 0x1af,
    KeySpellCheck = 0x1b0,
    KeyLogoff = 0x1b1,
    KeyDollar = 0x1b2,
    KeyEuro = 0x1b3,
    KeyFrameBack = 0x1b4,
    KeyFrameForward = 0x1b5,
    KeyContextMenu = 0x1b6,
    KeyMediaRepeat = 0x1b7,
    Key10ChannelsUp = 0x1b8,
    Key10ChannelsDown = 0x1b9,
    KeyImages = 0x1ba,
    KeyNotificationCenter = 0x1bc,
    KeyPickupPhone = 0x1bd,
    KeyHangupPhone = 0x1be,

    KeyDelEol = 0x1c0,
    KeyDelEos = 0x1c1,
    KeyInsLine = 0x1c2,
    KeyDelLine = 0x1c3,

    KeyFn = 0x1d0,
    KeyFnEsc = 0x1d1,
    KeyFnF1 = 0x1d2,
    KeyFnF2 = 0x1d3,
    KeyFnF3 = 0x1d4,
    KeyFnF4 = 0x1d5,
    KeyFnF5 = 0x1d6,
    KeyFnF6 = 0x1d7,
    KeyFnF7 = 0x1d8,
    KeyFnF8 = 0x1d9,
    KeyFnF9 = 0x1da,
    KeyFnF10 = 0x1db,
    KeyFnF11 = 0x1dc,
    KeyFnF12 = 0x1dd,
    KeyFn1 = 0x1de,
    KeyFn2 = 0x1df,
    KeyFnD = 0x1e0,
    KeyFnE = 0x1e1,
    KeyFnF = 0x1e2,
    KeyFnS = 0x1e3,
    KeyFnB = 0x1e4,
    KeyFnRightShift = 0x1e5,

    KeyBrlDot1 = 0x1f1,
    KeyBrlDot2 = 0x1f2,
    KeyBrlDot3 = 0x1f3,
    KeyBrlDot4 = 0x1f4,
    KeyBrlDot5 = 0x1f5,
    KeyBrlDot6 = 0x1f6,
    KeyBrlDot7 = 0x1f7,
    KeyBrlDot8 = 0x1f8,
    KeyBrlDot9 = 0x1f9,
    KeyBrlDot10 = 0x1fa,

    KeyNumeric0 = 0x200,
    KeyNumeric1 = 0x201,
    KeyNumeric2 = 0x202,
    KeyNumeric3 = 0x203,
    KeyNumeric4 = 0x204,
    KeyNumeric5 = 0x205,
    KeyNumeric6 = 0x206,
    KeyNumeric7 = 0x207,
    KeyNumeric8 = 0x208,
    KeyNumeric9 = 0x209,
    KeyNumericStar = 0x20a,
    KeyNumericPound = 0x20b,
    KeyNumericA = 0x20c,
    KeyNumericB = 0x20d,
    KeyNumericC = 0x20e,
    KeyNumericD = 0x20f,

    KeyCameraFocus = 0x210,
    KeyWpsButton = 0x211,

    KeyTouchpadToggle = 0x212,
    KeyTouchpadOn = 0x213,
    KeyTouchpadOff = 0x214,

    KeyCameraZoomIn = 0x215,
    KeyCameraZoomOut = 0x216,
    KeyCameraUp = 0x217,
    KeyCameraDown = 0x218,
    KeyCameraLeft = 0x219,
    KeyCameraRight = 0x21a,

    KeyAttendantOn = 0x21b,
    KeyAttendantOff = 0x21c,
    KeyAttendantToggle = 0x21d,
    KeyLightsToggle = 0x21e,

    BtnDpadUp = 0x220,
    BtnDpadDown = 0x221,
    BtnDpadLeft = 0x222,
    BtnDpadRight = 0x223,

    KeyAlsToggle = 0x230,
    KeyRotateLockToggle = 0x231,

    KeyButtonConfig = 0x240,
    KeyTaskManager = 0x241,
    KeyJournal = 0x242,
    KeyControlPanel = 0x243,
    KeyAppSelect = 0x244,
    KeyScreenSaver = 0x245,
    KeyVoiceCommand = 0x246,
    KeyAssistant = 0x247,
    KeyKbdLayoutNext = 0x248,
    KeyEmojiPicker = 0x249,
    KeyDictate = 0x24a,

    KeyBrightnessMin = 0x250,
    KeyBrightnessMax = 0x251,

    KeyKbdInputAssistPrev = 0x260,
    KeyKbdInputAssistNext = 0x261,
    KeyKbdInputAssistPrevGroup = 0x262,
    KeyKbdInputAssistNextGroup = 0x263,
    KeyKbdInputAssistAccept = 0x264,
    KeyKbdInputAssistCancel = 0x265,

    KeyRightUp = 0x266,
    KeyRightDown = 0x267,
    KeyLeftUp = 0x268,
    KeyLeftDown = 0x269,

    KeyRootMenu = 0x26a,
    KeyMediaTopMenu = 0x26b,
    KeyNumeric11 = 0x26c,
    KeyNumeric12 = 0x26d,

    KeyAudioDesc = 0x26e,
    Key3DMode = 0x26f,
    KeyNextFavorite = 0x270,
    KeyStopRecord = 0x271,
    KeyPauseRecord = 0x272,
    KeyVod = 0x273,
    KeyUnmute = 0x274,
    KeyFastReverse = 0x275,
    KeySlowReverse = 0x276,
    KeyData = 0x277,
    KeyOnscreenKeyboard = 0x278,
    KeyPrivacyScreenToggle = 0x279,
    KeySelectiveScreenshot = 0x27a,

    KeyMacro1 = 0x290,
    KeyMacro2 = 0x291,
    KeyMacro3 = 0x292,
    KeyMacro4 = 0x293,
    KeyMacro5 = 0x294,
    KeyMacro6 = 0x295,
    KeyMacro7 = 0x296,
    KeyMacro8 = 0x297,
    KeyMacro9 = 0x298,
    KeyMacro10 = 0x299,
    KeyMacro11 = 0x29a,
    KeyMacro12 = 0x29b,
    KeyMacro13 = 0x29c,
    KeyMacro14 = 0x29d,
    KeyMacro15 = 0x29e,
    KeyMacro16 = 0x29f,
    KeyMacro17 = 0x2a0,
    KeyMacro18 = 0x2a1,
    KeyMacro19 = 0x2a2,
    KeyMacro20 = 0x2a3,
    KeyMacro21 = 0x2a4,
    KeyMacro22 = 0x2a5,
    KeyMacro23 = 0x2a6,
    KeyMacro24 = 0x2a7,
    KeyMacro25 = 0x2a8,
    KeyMacro26 = 0x2a9,
    KeyMacro27 = 0x2aa,
    KeyMacro28 = 0x2ab,
    KeyMacro29 = 0x2ac,
    KeyMacro30 = 0x2ad,

    KeyMacroRecordStart = 0x2b0,
    KeyMacroRecordStop = 0x2b1,
    KeyMacroPresetCycle = 0x2b2,
    KeyMacroPreset1 = 0x2b3,
    KeyMacroPreset2 = 0x2b4,
    KeyMacroPreset3 = 0x2b5,

    KeyKbdLcdMenu1 = 0x2b8,
    KeyKbdLcdMenu2 = 0x2b9,
    KeyKbdLcdMenu3 = 0x2ba,
    KeyKbdLcdMenu4 = 0x2bb,
    KeyKbdLcdMenu5 = 0x2bc,

    BtnTriggerHappy1 = 0x2c0,
    BtnTriggerHappy2 = 0x2c1,
    BtnTriggerHappy3 = 0x2c2,
    BtnTriggerHappy4 = 0x2c3,
    BtnTriggerHappy5 = 0x2c4,
    BtnTriggerHappy6 = 0x2c5,
    BtnTriggerHappy7 = 0x2c6,
    BtnTriggerHappy8 = 0x2c7,
    BtnTriggerHappy9 = 0x2c8,
    BtnTriggerHappy10 = 0x2c9,
    BtnTriggerHappy11 = 0x2ca,
    BtnTriggerHappy12 = 0x2cb,
    BtnTriggerHappy13 = 0x2cc,
    BtnTriggerHappy14 = 0x2cd,
    BtnTriggerHappy15 = 0x2ce,
    BtnTriggerHappy16 = 0x2cf,
    BtnTriggerHappy17 = 0x2d0,
    BtnTriggerHappy18 = 0x2d1,
    BtnTriggerHappy19 = 0x2d2,
    BtnTriggerHappy20 = 0x2d3,
    BtnTriggerHappy21 = 0x2d4,
    BtnTriggerHappy22 = 0x2d5,
    BtnTriggerHappy23 = 0x2d6,
    BtnTriggerHappy24 = 0x2d7,
    BtnTriggerHappy25 = 0x2d8,
    BtnTriggerHappy26 = 0x2d9,
    BtnTriggerHappy27 = 0x2da,
    BtnTriggerHappy28 = 0x2db,
    BtnTriggerHappy29 = 0x2dc,
    BtnTriggerHappy30 = 0x2dd,
    BtnTriggerHappy31 = 0x2de,
    BtnTriggerHappy32 = 0x2df,
    BtnTriggerHappy33 = 0x2e0,
    BtnTriggerHappy34 = 0x2e1,
    BtnTriggerHappy35 = 0x2e2,
    BtnTriggerHappy36 = 0x2e3,
    BtnTriggerHappy37 = 0x2e4,
    BtnTriggerHappy38 = 0x2e5,
    BtnTriggerHappy39 = 0x2e6,
    BtnTriggerHappy40 = 0x2e7,
}
// Maximum value for key events
const KEY_MAX: usize = 0x2ff;
pub const KEY_COUNT: usize = KEY_MAX + 1;

const FF_MAX: usize = 0x7f;
pub const FF_COUNT: usize = FF_MAX + 1;