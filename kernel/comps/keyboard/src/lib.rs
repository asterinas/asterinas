// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};

use component::{init_component, ComponentInitError};
use ostd::sync::{LocalIrqDisabled, SpinLock};

#[cfg(target_arch = "x86_64")]
mod i8042_chip;

static KEYBOARD_CALLBACKS: SpinLock<Vec<Box<KeyboardCallback>>, LocalIrqDisabled> =
    SpinLock::new(Vec::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    #[cfg(target_arch = "x86_64")]
    i8042_chip::init();
    Ok(())
}

/// The callback function for keyboard.
pub type KeyboardCallback = dyn Fn(InputKey) + Send + Sync;

pub fn register_callback(callback: &'static KeyboardCallback) {
    KEYBOARD_CALLBACKS.lock().push(Box::new(callback));
}

/// Define unified keycodes for different types of keyboards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKey {
    // Control characters
    Nul,          // Ctrl + @, null
    Soh,          // Ctrl + A, start of heading
    Stx,          // Ctrl + B, start of text
    Etx,          // Ctrl + C, end of text
    Eot,          // Ctrl + D, end of transmission
    Enq,          // Ctrl + E, enquiry
    Ack,          // Ctrl + F, acknowledge
    Bel,          // Ctrl + G, bell
    Bs,           // Ctrl + H, backspace,
    Tab,          // Ctrl + I, horizontal tab
    Lf,           // Ctrl + J, NL line feed, new line
    Vt,           // Ctrl + K, vertical tab
    Ff,           // Ctrl + L, NP form feed, new page
    Cr,           // Ctrl + M, carriage return
    So,           // Ctrl + N, shift out
    Si,           // Ctrl + O, shift in
    Dle,          // Ctrl + P, data link escape
    Dc1,          // Ctrl + Q, device control 1
    Dc2,          // Ctrl + R, device control 2
    Dc3,          // Ctrl + S, device control 3
    Dc4,          // Ctrl + T, device control 4
    Nak,          // Ctrl + U, negative acknowledge
    Syn,          // Ctrl + V, synchronous idle
    Etb,          // Ctrl + W, end of trans. block
    Can,          // Ctrl + X, cancel
    Em,           // Ctrl + Y, end of medium
    Sub,          // Ctrl + Z, substitute
    Esc,          // Ctrl + [, escape
    Fs,           // Ctrl + \, file separator
    Gs,           // Ctrl + ], group separator
    Rs,           // Ctrl + ^, record separator
    Us,           // Ctrl + _, unit separator
    Space,        // ' '
    Exclamation,  // '!'
    DoubleQuote,  // '"'
    Hash,         // '#'
    Dollar,       // '$'
    Percent,      // '%'
    Ampersand,    // '&'
    SingleQuote,  // '''
    LeftParen,    // '('
    RightParen,   // ')'
    Asterisk,     // '*'
    Plus,         // '+'
    Comma,        // ','
    Minus,        // '-'
    Period,       // '.'
    ForwardSlash, // '/'
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Colon,       // ':'
    SemiColon,   // ';'
    LessThan,    // '<'
    Equal,       // '='
    GreaterThan, // '>'
    Question,    // '?'
    At,          // '@'
    UppercaseA,
    UppercaseB,
    UppercaseC,
    UppercaseD,
    UppercaseE,
    UppercaseF,
    UppercaseG,
    UppercaseH,
    UppercaseI,
    UppercaseJ,
    UppercaseK,
    UppercaseL,
    UppercaseM,
    UppercaseN,
    UppercaseO,
    UppercaseP,
    UppercaseQ,
    UppercaseR,
    UppercaseS,
    UppercaseT,
    UppercaseU,
    UppercaseV,
    UppercaseW,
    UppercaseX,
    UppercaseY,
    UppercaseZ,
    LeftBracket,  // '['
    BackSlash,    // '\'
    RightBracket, // ']'
    Caret,        // '^'
    Underscore,   // '_'
    Backtick,     // '`'
    LowercaseA,
    LowercaseB,
    LowercaseC,
    LowercaseD,
    LowercaseE,
    LowercaseF,
    LowercaseG,
    LowercaseH,
    LowercaseI,
    LowercaseJ,
    LowercaseK,
    LowercaseL,
    LowercaseM,
    LowercaseN,
    LowercaseO,
    LowercaseP,
    LowercaseQ,
    LowercaseR,
    LowercaseS,
    LowercaseT,
    LowercaseU,
    LowercaseV,
    LowercaseW,
    LowercaseX,
    LowercaseY,
    LowercaseZ,
    LeftBrace,  // '{'
    Pipe,       // '|'
    RightBrace, // '}'
    Tilde,      // '~'
    Del,
    UpArrow,
    DownArrow,
    RightArrow,
    LeftArrow,
    End,
    Home,
    Insert,
    Delete,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl InputKey {
    /// Gets the xterm control sequence for this key.
    ///
    /// Reference: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf>
    pub fn as_xterm_control_sequence(&self) -> &[u8] {
        match self {
            // ASCII control characters (character code 0-31)
            InputKey::Nul => &[0x00],
            InputKey::Soh => &[0x01],
            InputKey::Stx => &[0x02],
            InputKey::Etx => &[0x03],
            InputKey::Eot => &[0x04],
            InputKey::Enq => &[0x05],
            InputKey::Ack => &[0x06],
            InputKey::Bel => &[0x07],
            InputKey::Bs => &[0x08],
            InputKey::Tab => &[0x09],
            InputKey::Lf => &[0x0A],
            InputKey::Vt => &[0x0B],
            InputKey::Ff => &[0x0C],
            InputKey::Cr => &[0x0D],
            InputKey::So => &[0x0E],
            InputKey::Si => &[0x0F],
            InputKey::Dle => &[0x10],
            InputKey::Dc1 => &[0x11],
            InputKey::Dc2 => &[0x12],
            InputKey::Dc3 => &[0x13],
            InputKey::Dc4 => &[0x14],
            InputKey::Nak => &[0x15],
            InputKey::Syn => &[0x16],
            InputKey::Etb => &[0x17],
            InputKey::Can => &[0x18],
            InputKey::Em => &[0x19],
            InputKey::Sub => &[0x1A],
            InputKey::Esc => &[0x1B],
            InputKey::Fs => &[0x1C],
            InputKey::Gs => &[0x1D],
            InputKey::Rs => &[0x1E],
            InputKey::Us => &[0x1F],
            // ASCII printable characters (character code 32-127)
            InputKey::Space => b" ",
            InputKey::Exclamation => b"!",
            InputKey::DoubleQuote => b"\"",
            InputKey::Hash => b"#",
            InputKey::Dollar => b"$",
            InputKey::Percent => b"%",
            InputKey::Ampersand => b"&",
            InputKey::SingleQuote => b"'",
            InputKey::LeftParen => b"(",
            InputKey::RightParen => b")",
            InputKey::Asterisk => b"*",
            InputKey::Plus => b"+",
            InputKey::Comma => b",",
            InputKey::Minus => b"-",
            InputKey::Period => b".",
            InputKey::ForwardSlash => b"/",
            InputKey::Zero => b"0",
            InputKey::One => b"1",
            InputKey::Two => b"2",
            InputKey::Three => b"3",
            InputKey::Four => b"4",
            InputKey::Five => b"5",
            InputKey::Six => b"6",
            InputKey::Seven => b"7",
            InputKey::Eight => b"8",
            InputKey::Nine => b"9",
            InputKey::Colon => b":",
            InputKey::SemiColon => b";",
            InputKey::LessThan => b"<",
            InputKey::Equal => b"=",
            InputKey::GreaterThan => b">",
            InputKey::Question => b"?",
            InputKey::At => b"@",
            InputKey::UppercaseA => b"A",
            InputKey::UppercaseB => b"B",
            InputKey::UppercaseC => b"C",
            InputKey::UppercaseD => b"D",
            InputKey::UppercaseE => b"E",
            InputKey::UppercaseF => b"F",
            InputKey::UppercaseG => b"G",
            InputKey::UppercaseH => b"H",
            InputKey::UppercaseI => b"I",
            InputKey::UppercaseJ => b"J",
            InputKey::UppercaseK => b"K",
            InputKey::UppercaseL => b"L",
            InputKey::UppercaseM => b"M",
            InputKey::UppercaseN => b"N",
            InputKey::UppercaseO => b"O",
            InputKey::UppercaseP => b"P",
            InputKey::UppercaseQ => b"Q",
            InputKey::UppercaseR => b"R",
            InputKey::UppercaseS => b"S",
            InputKey::UppercaseT => b"T",
            InputKey::UppercaseU => b"U",
            InputKey::UppercaseV => b"V",
            InputKey::UppercaseW => b"W",
            InputKey::UppercaseX => b"X",
            InputKey::UppercaseY => b"Y",
            InputKey::UppercaseZ => b"Z",
            InputKey::LeftBracket => b"[",
            InputKey::BackSlash => b"\\",
            InputKey::RightBracket => b"]",
            InputKey::Caret => b"^",
            InputKey::Underscore => b"_",
            InputKey::Backtick => b"`",
            InputKey::LowercaseA => b"a",
            InputKey::LowercaseB => b"b",
            InputKey::LowercaseC => b"c",
            InputKey::LowercaseD => b"d",
            InputKey::LowercaseE => b"e",
            InputKey::LowercaseF => b"f",
            InputKey::LowercaseG => b"g",
            InputKey::LowercaseH => b"h",
            InputKey::LowercaseI => b"i",
            InputKey::LowercaseJ => b"j",
            InputKey::LowercaseK => b"k",
            InputKey::LowercaseL => b"l",
            InputKey::LowercaseM => b"m",
            InputKey::LowercaseN => b"n",
            InputKey::LowercaseO => b"o",
            InputKey::LowercaseP => b"p",
            InputKey::LowercaseQ => b"q",
            InputKey::LowercaseR => b"r",
            InputKey::LowercaseS => b"s",
            InputKey::LowercaseT => b"t",
            InputKey::LowercaseU => b"u",
            InputKey::LowercaseV => b"v",
            InputKey::LowercaseW => b"w",
            InputKey::LowercaseX => b"x",
            InputKey::LowercaseY => b"y",
            InputKey::LowercaseZ => b"z",
            InputKey::LeftBrace => b"{",
            InputKey::Pipe => b"|",
            InputKey::RightBrace => b"}",
            InputKey::Tilde => b"~",
            InputKey::Del => &[0x7F],
            // PC-Style Function Keys
            InputKey::UpArrow => b"\x1B[A",
            InputKey::DownArrow => b"\x1B[B",
            InputKey::RightArrow => b"\x1B[C",
            InputKey::LeftArrow => b"\x1B[D",
            InputKey::End => b"\x1B[F",
            InputKey::Home => b"\x1B[H",
            InputKey::Insert => b"\x1B[2~",
            InputKey::Delete => b"\x1B[3~",
            InputKey::PageUp => b"\x1B[5~",
            InputKey::PageDown => b"\x1B[6~",
            InputKey::F1 => b"\x1BOP",
            InputKey::F2 => b"\x1BOQ",
            InputKey::F3 => b"\x1BOR",
            InputKey::F4 => b"\x1BOS",
            InputKey::F5 => b"\x1B[15~",
            InputKey::F6 => b"\x1B[17~",
            InputKey::F7 => b"\x1B[18~",
            InputKey::F8 => b"\x1B[19~",
            InputKey::F9 => b"\x1B[20~",
            InputKey::F10 => b"\x1B[21~",
            InputKey::F11 => b"\x1B[23~",
            InputKey::F12 => b"\x1B[24~",
        }
    }
}
