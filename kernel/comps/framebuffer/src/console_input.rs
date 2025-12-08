// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass},
};

use crate::FRAMEBUFFER_CONSOLE;

#[derive(Debug, Clone)]
struct FbConsoleHandlerClass;

impl InputHandlerClass for FbConsoleHandlerClass {
    fn name(&self) -> &str {
        "fb_console"
    }

    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError> {
        let capability = dev.capability();
        if !capability.look_like_keyboard() {
            return Err(ConnectError::IncompatibleDevice);
        }
        log::info!(
            "Framebuffer console handler connected to device: {}",
            dev.name()
        );
        Ok(Arc::new(FbConsoleHandler::new()))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        log::info!(
            "Framebuffer console handler disconnected from device: {}",
            dev.name()
        );
    }
}

/// Framebuffer console handler instance for a specific input device.
#[derive(Debug)]
struct FbConsoleHandler {
    shift_pressed: AtomicBool,
    ctrl_pressed: AtomicBool,
    caps_lock: AtomicBool,
}

impl FbConsoleHandler {
    fn new() -> Self {
        Self {
            shift_pressed: AtomicBool::new(false),
            ctrl_pressed: AtomicBool::new(false),
            caps_lock: AtomicBool::new(false),
        }
    }

    /// Converts a `KeyCode` to an ASCII character or an xterm control sequence.
    ///
    /// Reference: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf>
    fn keycode_to_ascii(&self, keycode: KeyCode) -> Option<&'static [u8]> {
        let shift = self.shift_pressed.load(Ordering::Relaxed);
        if self.ctrl_pressed.load(Ordering::Relaxed) {
            return match keycode {
                KeyCode::Num2 if shift => Some(b"\x00"), // Ctrl + @, null (NUL)
                KeyCode::A => Some(b"\x01"),             // Ctrl + A, start of heading (SOH)
                KeyCode::B => Some(b"\x02"),             // Ctrl + B, start of text (STX)
                KeyCode::C => Some(b"\x03"),             // Ctrl + C, end of text (ETX)
                KeyCode::D => Some(b"\x04"),             // Ctrl + D, end of transmission (EOT)
                KeyCode::E => Some(b"\x05"),             // Ctrl + E, enquiry (ENQ)
                KeyCode::F => Some(b"\x06"),             // Ctrl + F, acknowledge (ACK)
                KeyCode::G => Some(b"\x07"),             // Ctrl + G, bell (BEL)
                KeyCode::H => Some(b"\x08"),             // Ctrl + H, backspace (BS)
                KeyCode::I => Some(b"\t"),               // Ctrl + I, horizontal tab (TAB)
                KeyCode::J => Some(b"\n"),               // Ctrl + J, line feed/new line (LF)
                KeyCode::K => Some(b"\x0b"),             // Ctrl + K, vertical tab (VT)
                KeyCode::L => Some(b"\x0c"),             // Ctrl + L, form feed/new page (FF)
                KeyCode::M => Some(b"\r"),               // Ctrl + M, carriage return (CR)
                KeyCode::N => Some(b"\x0e"),             // Ctrl + N, shift out (SO)
                KeyCode::O => Some(b"\x0f"),             // Ctrl + O, shift in (SI)
                KeyCode::P => Some(b"\x10"),             // Ctrl + P, data link escape (DLE)
                KeyCode::Q => Some(b"\x11"),             // Ctrl + Q, device control 1 (DC1)
                KeyCode::R => Some(b"\x12"),             // Ctrl + R, device control 2 (DC2)
                KeyCode::S => Some(b"\x13"),             // Ctrl + S, device control 3 (DC3)
                KeyCode::T => Some(b"\x14"),             // Ctrl + T, device control 4 (DC4)
                KeyCode::U => Some(b"\x15"),             // Ctrl + U, negative acknowledge (NAK)
                KeyCode::V => Some(b"\x16"),             // Ctrl + V, synchronous idle (SYN)
                KeyCode::W => Some(b"\x17"),             // Ctrl + W, end of trans. block (ETB)
                KeyCode::X => Some(b"\x18"),             // Ctrl + X, cancel (CAN)
                KeyCode::Y => Some(b"\x19"),             // Ctrl + Y, end of medium (EM)
                KeyCode::Z => Some(b"\x1a"),             // Ctrl + Z, substitute (SUB)
                KeyCode::LeftBrace => Some(b"\x1b"),     // Ctrl + [, escape (ESC)
                KeyCode::Backslash => Some(b"\\"),       // Ctrl + \, file separator (FS)
                KeyCode::RightBrace => Some(b"\x1d"),    // Ctrl + ], group separator (GS)
                KeyCode::Num6 if shift => Some(b"\x1e"), // Ctrl + ^, record separator (RS)
                KeyCode::Minus if shift => Some(b"\x1f"), // Ctrl + _, unit separator (US)
                _ => None,
            };
        }

        let caps_lock = self.caps_lock.load(Ordering::Relaxed);
        match keycode {
            // Letters
            KeyCode::A => Some(if shift ^ caps_lock { b"A" } else { b"a" }),
            KeyCode::B => Some(if shift ^ caps_lock { b"B" } else { b"b" }),
            KeyCode::C => Some(if shift ^ caps_lock { b"C" } else { b"c" }),
            KeyCode::D => Some(if shift ^ caps_lock { b"D" } else { b"d" }),
            KeyCode::E => Some(if shift ^ caps_lock { b"E" } else { b"e" }),
            KeyCode::F => Some(if shift ^ caps_lock { b"F" } else { b"f" }),
            KeyCode::G => Some(if shift ^ caps_lock { b"G" } else { b"g" }),
            KeyCode::H => Some(if shift ^ caps_lock { b"H" } else { b"h" }),
            KeyCode::I => Some(if shift ^ caps_lock { b"I" } else { b"i" }),
            KeyCode::J => Some(if shift ^ caps_lock { b"J" } else { b"j" }),
            KeyCode::K => Some(if shift ^ caps_lock { b"K" } else { b"k" }),
            KeyCode::L => Some(if shift ^ caps_lock { b"L" } else { b"l" }),
            KeyCode::M => Some(if shift ^ caps_lock { b"M" } else { b"m" }),
            KeyCode::N => Some(if shift ^ caps_lock { b"N" } else { b"n" }),
            KeyCode::O => Some(if shift ^ caps_lock { b"O" } else { b"o" }),
            KeyCode::P => Some(if shift ^ caps_lock { b"P" } else { b"p" }),
            KeyCode::Q => Some(if shift ^ caps_lock { b"Q" } else { b"q" }),
            KeyCode::R => Some(if shift ^ caps_lock { b"R" } else { b"r" }),
            KeyCode::S => Some(if shift ^ caps_lock { b"S" } else { b"s" }),
            KeyCode::T => Some(if shift ^ caps_lock { b"T" } else { b"t" }),
            KeyCode::U => Some(if shift ^ caps_lock { b"U" } else { b"u" }),
            KeyCode::V => Some(if shift ^ caps_lock { b"V" } else { b"v" }),
            KeyCode::W => Some(if shift ^ caps_lock { b"W" } else { b"w" }),
            KeyCode::X => Some(if shift ^ caps_lock { b"X" } else { b"x" }),
            KeyCode::Y => Some(if shift ^ caps_lock { b"Y" } else { b"y" }),
            KeyCode::Z => Some(if shift ^ caps_lock { b"Z" } else { b"z" }),

            // Numbers
            KeyCode::Num0 => Some(if shift { b")" } else { b"0" }),
            KeyCode::Num1 => Some(if shift { b"!" } else { b"1" }),
            KeyCode::Num2 => Some(if shift { b"@" } else { b"2" }),
            KeyCode::Num3 => Some(if shift { b"#" } else { b"3" }),
            KeyCode::Num4 => Some(if shift { b"$" } else { b"4" }),
            KeyCode::Num5 => Some(if shift { b"%" } else { b"5" }),
            KeyCode::Num6 => Some(if shift { b"^" } else { b"6" }),
            KeyCode::Num7 => Some(if shift { b"&" } else { b"7" }),
            KeyCode::Num8 => Some(if shift { b"*" } else { b"8" }),
            KeyCode::Num9 => Some(if shift { b"(" } else { b"9" }),

            // Special characters
            KeyCode::Space => Some(b" "),
            KeyCode::Enter => Some(b"\n"),
            KeyCode::Tab => Some(b"\t"),
            KeyCode::Backspace => Some(b"\x08"),
            KeyCode::Esc => Some(b"\x1b"),
            KeyCode::Delete => Some(b"\x7f"),

            // Punctuation
            KeyCode::Minus => Some(if shift { b"_" } else { b"-" }),
            KeyCode::Equal => Some(if shift { b"+" } else { b"=" }),
            KeyCode::LeftBrace => Some(if shift { b"{" } else { b"[" }),
            KeyCode::RightBrace => Some(if shift { b"}" } else { b"]" }),
            KeyCode::Backslash => Some(if shift { b"|" } else { b"\\" }),
            KeyCode::Semicolon => Some(if shift { b":" } else { b";" }),
            KeyCode::Apostrophe => Some(if shift { b"\"" } else { b"'" }),
            KeyCode::Grave => Some(if shift { b"~" } else { b"`" }),
            KeyCode::Comma => Some(if shift { b"<" } else { b"," }),
            KeyCode::Dot => Some(if shift { b">" } else { b"." }),
            KeyCode::Slash => Some(if shift { b"?" } else { b"/" }),

            // Function keys (F1-F12)
            KeyCode::F1 => Some(b"\x1b[11~"),
            KeyCode::F2 => Some(b"\x1b[12~"),
            KeyCode::F3 => Some(b"\x1b[13~"),
            KeyCode::F4 => Some(b"\x1b[14~"),
            KeyCode::F5 => Some(b"\x1b[15~"),
            KeyCode::F6 => Some(b"\x1b[17~"),
            KeyCode::F7 => Some(b"\x1b[18~"),
            KeyCode::F8 => Some(b"\x1b[19~"),
            KeyCode::F9 => Some(b"\x1b[20~"),
            KeyCode::F10 => Some(b"\x1b[21~"),
            KeyCode::F11 => Some(b"\x1b[23~"),
            KeyCode::F12 => Some(b"\x1b[24~"),

            // Arrow keys
            KeyCode::Up => Some(b"\x1b[A"),
            KeyCode::Down => Some(b"\x1b[B"),
            KeyCode::Right => Some(b"\x1b[C"),
            KeyCode::Left => Some(b"\x1b[D"),

            // Navigation keys
            KeyCode::Home => Some(b"\x1b[H"),
            KeyCode::End => Some(b"\x1b[F"),
            KeyCode::PageUp => Some(b"\x1b[5~"),
            KeyCode::PageDown => Some(b"\x1b[6~"),
            KeyCode::Insert => Some(b"\x1b[2~"),

            _ => None,
        }
    }

    fn handle_key_event(&self, keycode: KeyCode, key_status: KeyStatus) {
        log::trace!(
            "Framebuffer console handler received key event: {:?} {:?}",
            keycode,
            key_status
        );

        match keycode {
            KeyCode::LeftShift | KeyCode::RightShift => {
                let is_pressed = key_status == KeyStatus::Pressed;
                self.shift_pressed.store(is_pressed, Ordering::Relaxed);
                return;
            }
            KeyCode::LeftCtrl | KeyCode::RightCtrl => {
                let is_pressed = key_status == KeyStatus::Pressed;
                self.ctrl_pressed.store(is_pressed, Ordering::Relaxed);
                return;
            }
            KeyCode::CapsLock => {
                if key_status == KeyStatus::Pressed {
                    let new_caps = !self.caps_lock.load(Ordering::Relaxed);
                    self.caps_lock.store(new_caps, Ordering::Relaxed);
                }
                return;
            }
            _ => {}
        }

        if key_status == KeyStatus::Released {
            return;
        }

        if let Some(bytes) = self.keycode_to_ascii(keycode)
            && let Some(console) = FRAMEBUFFER_CONSOLE.get()
        {
            console.trigger_input_callbacks(bytes);
        }
    }
}

impl InputHandler for FbConsoleHandler {
    fn handle_events(&self, events: &[InputEvent]) {
        for event in events {
            match event {
                InputEvent::Key(keycode, key_status) => {
                    self.handle_key_event(*keycode, *key_status)
                }
                InputEvent::Sync(_) => {
                    // nothing to do
                }
                _ => {
                    log::warn!(
                        "Framebuffer console handler received unsupported event: {:?}",
                        event
                    );
                }
            }
        }
    }
}

pub(crate) fn init() {
    if FRAMEBUFFER_CONSOLE.get().is_none() {
        return;
    }
    let handler_class = Arc::new(FbConsoleHandlerClass);
    let registered = aster_input::register_handler_class(handler_class);

    core::mem::forget(registered);
}
