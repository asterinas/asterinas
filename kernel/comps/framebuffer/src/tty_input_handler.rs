// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass, RegisteredInputHandlerClass},
};
use spin::Once;

use crate::FRAMEBUFFER_CONSOLE;

#[derive(Debug, Clone)]
struct FbConsoleHandlerClass;

impl FbConsoleHandlerClass {}

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
        Ok(Arc::new(FbConsoleHandler::new(Arc::new(self.clone()))))
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
    handler_class: Arc<dyn InputHandlerClass>,
    shift_pressed: AtomicBool,
    caps_lock: AtomicBool,
}

impl FbConsoleHandler {
    fn new(handler_class: Arc<dyn InputHandlerClass>) -> Self {
        Self {
            handler_class,
            shift_pressed: AtomicBool::new(false),
            caps_lock: AtomicBool::new(false),
        }
    }

    /// Converts a `KeyCode` to an ASCII character or an xterm control sequence.
    ///
    /// Reference: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf>
    fn keycode_to_ascii(
        &self,
        keycode: KeyCode,
        shift: bool,
        caps_lock: bool,
    ) -> Option<&'static [u8]> {
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

        let shift_pressed = self.shift_pressed.load(Ordering::Relaxed);
        let caps_lock = self.caps_lock.load(Ordering::Relaxed);
        if let Some(bytes) = self.keycode_to_ascii(keycode, shift_pressed, caps_lock) {
            if let Some(console) = FRAMEBUFFER_CONSOLE.get() {
                console.trigger_input_callbacks(bytes);
            }
        }
    }
}

impl InputHandler for FbConsoleHandler {
    fn class_name(&self) -> &str {
        "fb_console"
    }

    fn handler_class(&self) -> &Arc<dyn InputHandlerClass> {
        &self.handler_class
    }

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

static FB_CONSOLE_HANDLER: Once<RegisteredInputHandlerClass> = Once::new();

pub(crate) fn init() {
    if FRAMEBUFFER_CONSOLE.get().is_none() {
        return;
    }
    let handler_class = Arc::new(FbConsoleHandlerClass);
    let registered = aster_input::register_handler_class(handler_class);
    FB_CONSOLE_HANDLER.call_once(|| registered);
}
