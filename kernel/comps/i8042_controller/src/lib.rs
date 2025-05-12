// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard and mouse input.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::ops::Deref;
use spin::Once;
use ostd::{
    arch::{device::io_port::ReadWriteAccess, IO_APIC},
    io::IoPort,
    sync::SpinLock,
    trap::{IrqLine, TrapFrame},
};
use core::hint::spin_loop;

use component::{init_component, ComponentInitError};

mod i8042_mouse;
mod i8042_keyboard;

use crate::i8042_keyboard::handle_keyboard_input;
use crate::i8042_mouse::handle_mouse_input;


static MOUSE_CALLBACKS: SpinLock<Vec<Box<MouseCallback>>> = SpinLock::new(Vec::new());

/// Data register (R/W)
static DATA_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// Status register (R)
static STATUS_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// IrqLine for i8042 keyboard.
static KEYBOARD_IRQ_LINE: Once<SpinLock<IrqLine>> = Once::new();

/// IrqLine for i8042 mouse.
static MOUSE_IRQ_LINE: Once<SpinLock<IrqLine>> = Once::new();

// Controller commands
const DISABLE_MOUSE: u8 = 0xA7;
const ENABLE_MOUSE: u8 = 0xA8;
const DISABLE_KEYBOARD: u8 = 0xAD;
const ENABLE_KEYBOARD: u8 = 0xAE;
const MOUSE_WRITE: u8 = 0xD4;
const READ_CONFIG: u8 = 0x20;
const WRITE_CONFIG: u8 = 0x60;

// Mouse commands
const MOUSE_ENABLE: u8 = 0xF4;
const MOUSE_RESET: u8 = 0xFF;
const MOUSE_DEFAULT: u8 = 0xF6;

// Configure bits
const ENABLE_KEYBOARD_BIT: u8 = 0x1;
const ENABLE_MOUSE_BIT: u8 = 0x2;
const ENABLE_MOUSE_CLOCK_BIT: u8 = 0x20;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    log::error!("This is init in kernel/comps/i8042_controller/lib.rs");

    DATA_PORT.call_once(|| IoPort::acquire(0x60).unwrap());
    STATUS_PORT.call_once(|| IoPort::acquire(0x64).unwrap());

    init_i8042_controller();
    init_mouse_device();


    let mut k_irq_line = IrqLine::alloc().unwrap();
    let mut m_irq_line = IrqLine::alloc().unwrap();
    k_irq_line.on_active(handle_keyboard_input);
    m_irq_line.on_active(handle_mouse_input);

    let mut io_apic = IO_APIC.get().unwrap()[0].lock();
    io_apic.enable(1, k_irq_line.clone()).unwrap();
    io_apic.enable(12, m_irq_line.clone()).unwrap();

    KEYBOARD_IRQ_LINE.call_once(|| {SpinLock::new(k_irq_line)});
    MOUSE_IRQ_LINE.call_once(|| {SpinLock::new(m_irq_line)});
    
    // init_mouse_device();

    i8042_keyboard::init();
    i8042_mouse::init();
    Ok(())
}


/// Initialize i8042 controller
fn init_i8042_controller() {
    // Disable keyborad and mouse
    STATUS_PORT.get().unwrap().write(DISABLE_MOUSE);
    STATUS_PORT.get().unwrap().write(DISABLE_KEYBOARD);

    // Clear the input buffer
    while DATA_PORT.get().unwrap().read() & 0x1 != 0 {
        let _ = DATA_PORT.get().unwrap().read();
    }

    // Set up the configuration
    STATUS_PORT.get().unwrap().write(READ_CONFIG); 
    let mut config = DATA_PORT.get().unwrap().read();
    config |= ENABLE_KEYBOARD_BIT; 
    config |= ENABLE_MOUSE_BIT; 
    config &= !ENABLE_MOUSE_CLOCK_BIT;

    STATUS_PORT.get().unwrap().write(WRITE_CONFIG);
    DATA_PORT.get().unwrap().write(config);

    // Enable keyboard and mouse
    STATUS_PORT.get().unwrap().write(ENABLE_KEYBOARD);
    STATUS_PORT.get().unwrap().write(ENABLE_MOUSE);
}

/// Initialize i8042 mouse
fn init_mouse_device() {
    // Send reset command
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_RESET);
    wait_ack();

    // Set up default configuration
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_DEFAULT);
    wait_ack();

    // Enable data reporting
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_ENABLE);
    wait_ack();
}

/// Wait for controller's acknowledgement
fn wait_ack() {
    loop {
        if STATUS_PORT.get().unwrap().read() & 0x1 != 0 {
            let data = DATA_PORT.get().unwrap().read();
            if data == 0xFA {
                return 
            }
        }
        spin_loop();
    }
}

/// The callback function for mouse.
pub type MouseCallback = dyn Fn() + Send + Sync;

pub fn mouse_register_callback(callback: &'static MouseCallback) {
    log::error!("This is register_callback in kernel/comps/mouse/src/lib.rs");
    MOUSE_CALLBACKS
        .disable_irq()
        .lock()
        .push(Box::new(callback));
}



static KEYBOARD_CALLBACKS: SpinLock<Vec<Box<KeyboardCallback>>> = SpinLock::new(Vec::new());

/// The callback function for keyboard.
pub type KeyboardCallback = dyn Fn(InputKey) + Send + Sync;

pub fn keyboard_register_callback(callback: &'static KeyboardCallback) {
    KEYBOARD_CALLBACKS
        .disable_irq()
        .lock()
        .push(Box::new(callback));
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
    // Escape sequences
    UpArrow,    // \x1B[A
    DownArrow,  // \x1B[B
    RightArrow, // \x1B[C
    LeftArrow,  // \x1B[D
    End,        // \x1B[F
    Home,       // \x1B[H
    Insert,     // \x1B[2~
    Delete,     // \x1B[3~
    PageUp,     // \x1B[5~
    PageDown,   // \x1B[6~
    F1,         // \x1BOP
    F2,         // \x1BOQ
    F3,         // \x1BOR
    F4,         // \x1BOS
    F5,         // \x1B[15~
    F6,         // \x1B[17~
    F7,         // \x1B[18~
    F8,         // \x1B[19~
    F9,         // \x1B[20~
    F10,        // \x1B[21~
    F11,        // \x1B[23~
    F12,        // \x1B[24~
}

impl Deref for InputKey {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
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
            InputKey::Space => &[0x20],
            InputKey::Exclamation => &[0x21],
            InputKey::DoubleQuote => &[0x22],
            InputKey::Hash => &[0x23],
            InputKey::Dollar => &[0x24],
            InputKey::Percent => &[0x25],
            InputKey::Ampersand => &[0x26],
            InputKey::SingleQuote => &[0x27],
            InputKey::LeftParen => &[0x28],
            InputKey::RightParen => &[0x29],
            InputKey::Asterisk => &[0x2A],
            InputKey::Plus => &[0x2B],
            InputKey::Comma => &[0x2C],
            InputKey::Minus => &[0x2D],
            InputKey::Period => &[0x2E],
            InputKey::ForwardSlash => &[0x2F],
            InputKey::Zero => &[0x30],
            InputKey::One => &[0x31],
            InputKey::Two => &[0x32],
            InputKey::Three => &[0x33],
            InputKey::Four => &[0x34],
            InputKey::Five => &[0x35],
            InputKey::Six => &[0x36],
            InputKey::Seven => &[0x37],
            InputKey::Eight => &[0x38],
            InputKey::Nine => &[0x39],
            InputKey::Colon => &[0x3A],
            InputKey::SemiColon => &[0x3B],
            InputKey::LessThan => &[0x3C],
            InputKey::Equal => &[0x3D],
            InputKey::GreaterThan => &[0x3E],
            InputKey::Question => &[0x3F],
            InputKey::At => &[0x40],
            InputKey::UppercaseA => &[0x41],
            InputKey::UppercaseB => &[0x42],
            InputKey::UppercaseC => &[0x43],
            InputKey::UppercaseD => &[0x44],
            InputKey::UppercaseE => &[0x45],
            InputKey::UppercaseF => &[0x46],
            InputKey::UppercaseG => &[0x47],
            InputKey::UppercaseH => &[0x48],
            InputKey::UppercaseI => &[0x49],
            InputKey::UppercaseJ => &[0x4A],
            InputKey::UppercaseK => &[0x4B],
            InputKey::UppercaseL => &[0x4C],
            InputKey::UppercaseM => &[0x4D],
            InputKey::UppercaseN => &[0x4E],
            InputKey::UppercaseO => &[0x4F],
            InputKey::UppercaseP => &[0x50],
            InputKey::UppercaseQ => &[0x51],
            InputKey::UppercaseR => &[0x52],
            InputKey::UppercaseS => &[0x53],
            InputKey::UppercaseT => &[0x54],
            InputKey::UppercaseU => &[0x55],
            InputKey::UppercaseV => &[0x56],
            InputKey::UppercaseW => &[0x57],
            InputKey::UppercaseX => &[0x58],
            InputKey::UppercaseY => &[0x59],
            InputKey::UppercaseZ => &[0x5A],
            InputKey::LeftBracket => &[0x5B],
            InputKey::BackSlash => &[0x5C],
            InputKey::RightBracket => &[0x5D],
            InputKey::Caret => &[0x5E],
            InputKey::Underscore => &[0x5F],
            InputKey::Backtick => &[0x60],
            InputKey::LowercaseA => &[0x61],
            InputKey::LowercaseB => &[0x62],
            InputKey::LowercaseC => &[0x63],
            InputKey::LowercaseD => &[0x64],
            InputKey::LowercaseE => &[0x65],
            InputKey::LowercaseF => &[0x66],
            InputKey::LowercaseG => &[0x67],
            InputKey::LowercaseH => &[0x68],
            InputKey::LowercaseI => &[0x69],
            InputKey::LowercaseJ => &[0x6A],
            InputKey::LowercaseK => &[0x6B],
            InputKey::LowercaseL => &[0x6C],
            InputKey::LowercaseM => &[0x6D],
            InputKey::LowercaseN => &[0x6E],
            InputKey::LowercaseO => &[0x6F],
            InputKey::LowercaseP => &[0x70],
            InputKey::LowercaseQ => &[0x71],
            InputKey::LowercaseR => &[0x72],
            InputKey::LowercaseS => &[0x73],
            InputKey::LowercaseT => &[0x74],
            InputKey::LowercaseU => &[0x75],
            InputKey::LowercaseV => &[0x76],
            InputKey::LowercaseW => &[0x77],
            InputKey::LowercaseX => &[0x78],
            InputKey::LowercaseY => &[0x79],
            InputKey::LowercaseZ => &[0x7A],
            InputKey::LeftBrace => &[0x7B],
            InputKey::Pipe => &[0x7C],
            InputKey::RightBrace => &[0x7D],
            InputKey::Tilde => &[0x7E],
            InputKey::Del => &[0x7F],
            InputKey::UpArrow => &[0x1B, 0x5B, 0x41],
            InputKey::DownArrow => &[0x1B, 0x5B, 0x42],
            InputKey::RightArrow => &[0x1B, 0x5B, 0x43],
            InputKey::LeftArrow => &[0x1B, 0x5B, 0x44],
            InputKey::End => &[0x1B, 0x5B, 0x46],
            InputKey::Home => &[0x1B, 0x5B, 0x48],
            InputKey::Insert => &[0x1B, 0x5B, 0x32, 0x7E],
            InputKey::Delete => &[0x1B, 0x5B, 0x33, 0x7E],
            InputKey::PageUp => &[0x1B, 0x5B, 0x35, 0x7E],
            InputKey::PageDown => &[0x1B, 0x5B, 0x36, 0x7E],
            InputKey::F1 => &[0x1B, 0x4F, 0x50],
            InputKey::F2 => &[0x1B, 0x4F, 0x51],
            InputKey::F3 => &[0x1B, 0x4F, 0x52],
            InputKey::F4 => &[0x1B, 0x4F, 0x53],
            InputKey::F5 => &[0x1B, 0x5B, 0x31, 0x35, 0x7E],
            InputKey::F6 => &[0x1B, 0x5B, 0x31, 0x37, 0x7E],
            InputKey::F7 => &[0x1B, 0x5B, 0x31, 0x38, 0x7E],
            InputKey::F8 => &[0x1B, 0x5B, 0x31, 0x39, 0x7E],
            InputKey::F9 => &[0x1B, 0x5B, 0x32, 0x30, 0x7E],
            InputKey::F10 => &[0x1B, 0x5B, 0x32, 0x31, 0x7E],
            InputKey::F11 => &[0x1B, 0x5B, 0x32, 0x33, 0x7E],
            InputKey::F12 => &[0x1B, 0x5B, 0x32, 0x34, 0x7E],
        }
    }
}
