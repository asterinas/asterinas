// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass, RegisteredInputHandlerClass},
};
use ostd::sync::SpinLock;
use spin::Once;

use crate::FRAMEBUFFER_CONSOLE;

#[derive(Debug)]
struct FbTtyHandlerClass {
    name: String,
}

impl FbTtyHandlerClass {
    pub fn new() -> Self {
        Self {
            name: "tty".to_string(),
        }
    }
}

impl InputHandlerClass for FbTtyHandlerClass {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError> {
        let capability = dev.capability();
        if !capability.look_like_keyboard() {
            return Err(ConnectError::IncompatibleDevice);
        }
        // Ensure framebuffer console exists
        let _ = FRAMEBUFFER_CONSOLE
            .get()
            .ok_or(ConnectError::InternalError)?;
        Ok(Arc::new(FbTtyHandler::new()))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        log::info!("TTY handler disconnected from device: {}", dev.name());
    }
}

/// TTY handler instance for a specific input device.
#[derive(Debug)]
struct FbTtyHandler {
    shift_pressed: AtomicBool,
    caps_lock: AtomicBool,
    pending_chars: SpinLock<Vec<u8>>,
}

impl FbTtyHandler {
    fn new() -> Self {
        Self {
            shift_pressed: AtomicBool::new(false),
            caps_lock: AtomicBool::new(false),
            pending_chars: SpinLock::new(Vec::with_capacity(256)),
        }
    }

    /// Converts a `KeyCode` to ASCII character or control sequence if possible.
    fn keycode_to_ascii(&self, keycode: KeyCode, shift: bool, caps_lock: bool) -> Option<Vec<u8>> {
        match keycode {
            // Letters
            KeyCode::KeyA => Some(vec![if shift ^ caps_lock { b'A' } else { b'a' }]),
            KeyCode::KeyB => Some(vec![if shift ^ caps_lock { b'B' } else { b'b' }]),
            KeyCode::KeyC => Some(vec![if shift ^ caps_lock { b'C' } else { b'c' }]),
            KeyCode::KeyD => Some(vec![if shift ^ caps_lock { b'D' } else { b'd' }]),
            KeyCode::KeyE => Some(vec![if shift ^ caps_lock { b'E' } else { b'e' }]),
            KeyCode::KeyF => Some(vec![if shift ^ caps_lock { b'F' } else { b'f' }]),
            KeyCode::KeyG => Some(vec![if shift ^ caps_lock { b'G' } else { b'g' }]),
            KeyCode::KeyH => Some(vec![if shift ^ caps_lock { b'H' } else { b'h' }]),
            KeyCode::KeyI => Some(vec![if shift ^ caps_lock { b'I' } else { b'i' }]),
            KeyCode::KeyJ => Some(vec![if shift ^ caps_lock { b'J' } else { b'j' }]),
            KeyCode::KeyK => Some(vec![if shift ^ caps_lock { b'K' } else { b'k' }]),
            KeyCode::KeyL => Some(vec![if shift ^ caps_lock { b'L' } else { b'l' }]),
            KeyCode::KeyM => Some(vec![if shift ^ caps_lock { b'M' } else { b'm' }]),
            KeyCode::KeyN => Some(vec![if shift ^ caps_lock { b'N' } else { b'n' }]),
            KeyCode::KeyO => Some(vec![if shift ^ caps_lock { b'O' } else { b'o' }]),
            KeyCode::KeyP => Some(vec![if shift ^ caps_lock { b'P' } else { b'p' }]),
            KeyCode::KeyQ => Some(vec![if shift ^ caps_lock { b'Q' } else { b'q' }]),
            KeyCode::KeyR => Some(vec![if shift ^ caps_lock { b'R' } else { b'r' }]),
            KeyCode::KeyS => Some(vec![if shift ^ caps_lock { b'S' } else { b's' }]),
            KeyCode::KeyT => Some(vec![if shift ^ caps_lock { b'T' } else { b't' }]),
            KeyCode::KeyU => Some(vec![if shift ^ caps_lock { b'U' } else { b'u' }]),
            KeyCode::KeyV => Some(vec![if shift ^ caps_lock { b'V' } else { b'v' }]),
            KeyCode::KeyW => Some(vec![if shift ^ caps_lock { b'W' } else { b'w' }]),
            KeyCode::KeyX => Some(vec![if shift ^ caps_lock { b'X' } else { b'x' }]),
            KeyCode::KeyY => Some(vec![if shift ^ caps_lock { b'Y' } else { b'y' }]),
            KeyCode::KeyZ => Some(vec![if shift ^ caps_lock { b'Z' } else { b'z' }]),

            // Numbers
            KeyCode::Key0 => Some(vec![if shift { b')' } else { b'0' }]),
            KeyCode::Key1 => Some(vec![if shift { b'!' } else { b'1' }]),
            KeyCode::Key2 => Some(vec![if shift { b'@' } else { b'2' }]),
            KeyCode::Key3 => Some(vec![if shift { b'#' } else { b'3' }]),
            KeyCode::Key4 => Some(vec![if shift { b'$' } else { b'4' }]),
            KeyCode::Key5 => Some(vec![if shift { b'%' } else { b'5' }]),
            KeyCode::Key6 => Some(vec![if shift { b'^' } else { b'6' }]),
            KeyCode::Key7 => Some(vec![if shift { b'&' } else { b'7' }]),
            KeyCode::Key8 => Some(vec![if shift { b'*' } else { b'8' }]),
            KeyCode::Key9 => Some(vec![if shift { b'(' } else { b'9' }]),

            // Special characters
            KeyCode::KeySpace => Some(vec![b' ']),
            KeyCode::KeyEnter => Some(vec![b'\n']),
            KeyCode::KeyTab => Some(vec![b'\t']),
            KeyCode::KeyBackspace => Some(vec![b'\x08']),
            KeyCode::KeyEsc => Some(vec![b'\x1b']),
            KeyCode::KeyDelete => Some(vec![b'\x7f']),

            // Punctuation
            KeyCode::KeyMinus => Some(vec![if shift { b'_' } else { b'-' }]),
            KeyCode::KeyEqual => Some(vec![if shift { b'+' } else { b'=' }]),
            KeyCode::KeyLeftBrace => Some(vec![if shift { b'{' } else { b'[' }]),
            KeyCode::KeyRightBrace => Some(vec![if shift { b'}' } else { b']' }]),
            KeyCode::KeyBackslash => Some(vec![if shift { b'|' } else { b'\\' }]),
            KeyCode::KeySemicolon => Some(vec![if shift { b':' } else { b';' }]),
            KeyCode::KeyApostrophe => Some(vec![if shift { b'\"' } else { b'\'' }]),
            KeyCode::KeyGrave => Some(vec![if shift { b'~' } else { b'`' }]),
            KeyCode::KeyComma => Some(vec![if shift { b'<' } else { b',' }]),
            KeyCode::KeyDot => Some(vec![if shift { b'>' } else { b'.' }]),
            KeyCode::KeySlash => Some(vec![if shift { b'?' } else { b'/' }]),

            // Function keys (F1-F12)
            KeyCode::KeyF1 => Some(b"\x1b[11~".to_vec()),
            KeyCode::KeyF2 => Some(b"\x1b[12~".to_vec()),
            KeyCode::KeyF3 => Some(b"\x1b[13~".to_vec()),
            KeyCode::KeyF4 => Some(b"\x1b[14~".to_vec()),
            KeyCode::KeyF5 => Some(b"\x1b[15~".to_vec()),
            KeyCode::KeyF6 => Some(b"\x1b[17~".to_vec()),
            KeyCode::KeyF7 => Some(b"\x1b[18~".to_vec()),
            KeyCode::KeyF8 => Some(b"\x1b[19~".to_vec()),
            KeyCode::KeyF9 => Some(b"\x1b[20~".to_vec()),
            KeyCode::KeyF10 => Some(b"\x1b[21~".to_vec()),
            KeyCode::KeyF11 => Some(b"\x1b[23~".to_vec()),
            KeyCode::KeyF12 => Some(b"\x1b[24~".to_vec()),

            // Arrow keys
            KeyCode::KeyUp => Some(b"\x1b[A".to_vec()),
            KeyCode::KeyDown => Some(b"\x1b[B".to_vec()),
            KeyCode::KeyRight => Some(b"\x1b[C".to_vec()),
            KeyCode::KeyLeft => Some(b"\x1b[D".to_vec()),

            // Navigation keys
            KeyCode::KeyHome => Some(b"\x1b[H".to_vec()),
            KeyCode::KeyEnd => Some(b"\x1b[F".to_vec()),
            KeyCode::KeyPageUp => Some(b"\x1b[5~".to_vec()),
            KeyCode::KeyPageDown => Some(b"\x1b[6~".to_vec()),
            KeyCode::KeyInsert => Some(b"\x1b[2~".to_vec()),

            _ => None,
        }
    }

    fn handle_key_event(&self, keycode: KeyCode, key_status: KeyStatus) {
        log::info!(
            "TTY handler received key event: {:?} {:?}",
            keycode,
            key_status
        );

        match keycode {
            KeyCode::KeyLeftShift | KeyCode::KeyRightShift => {
                let is_pressed = key_status == KeyStatus::Pressed;
                self.shift_pressed.store(is_pressed, Ordering::Relaxed);
                return;
            }
            KeyCode::KeyCapsLock => {
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
            let mut pending_chars = self.pending_chars.lock();
            pending_chars.extend(bytes.iter().copied());
        }
    }

    fn handle_sync_event(&self) {
        let mut pending_chars = self.pending_chars.lock();
        if !pending_chars.is_empty() {
            if let Some(console) = FRAMEBUFFER_CONSOLE.get() {
                aster_console::AnyConsoleDevice::send(&**console, &pending_chars);
            }
            pending_chars.clear();
        }
    }
}

impl InputHandler for FbTtyHandler {
    fn class_name(&self) -> &str {
        "tty"
    }

    fn handle_events(&self, events: &[InputEvent]) {
        for event in events {
            match event {
                InputEvent::Key(keycode, key_status) => {
                    self.handle_key_event(*keycode, *key_status)
                }
                InputEvent::Sync(_) => self.handle_sync_event(),
                _ => {
                    log::warn!("TTY handler received unsupported event: {:?}", event);
                }
            }
        }
    }
}

static FB_TTY_HANDLER: Once<RegisteredInputHandlerClass> = Once::new();

pub(crate) fn init() {
    if FRAMEBUFFER_CONSOLE.get().is_none() {
        return;
    }
    let handler_class = Arc::new(FbTtyHandlerClass::new());
    let registered = aster_input::register_handler_class(handler_class);
    FB_TTY_HANDLER.call_once(|| registered);
}
