// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::mode::{KeyboardMode, KeyboardModeFlags};
use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass},
};

use crate::device::tty::{
    Tty,
    vt::{
        VtDriver,
        keyboard::{
            CursorKey, LockKeyFlags, ModifierKey, ModifierKeyFlags, ModifierKeysState, NumpadKey,
            keysym::{FuncId, KeySym, SpecialHandler, get_func_bytes, get_keysym},
        },
        manager::{VIRTUAL_TERMINAL_MANAGER, active_vt},
    },
};

#[derive(Debug)]
struct VtKeyboardHandlerClass;

impl InputHandlerClass for VtKeyboardHandlerClass {
    fn name(&self) -> &str {
        "vt_keyboard"
    }

    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError> {
        let capability = dev.capability();
        if !capability.look_like_keyboard() {
            return Err(ConnectError::IncompatibleDevice);
        }
        log::info!(
            "Virtual terminal keyboard handler connected to device: {}",
            dev.name()
        );
        Ok(Arc::new(VtKeyboardHandler::new()))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        log::info!(
            "Virtual terminal keyboard handler disconnected from device: {}",
            dev.name()
        );
    }
}

/// Virtual terminal keyboard handler instance for a specific input device.
#[derive(Debug)]
struct VtKeyboardHandler {
    /// Current state of modifier keys.
    ///
    /// It is shared across all virtual terminals.
    modifier_state: ModifierKeysState,
}

impl VtKeyboardHandler {
    fn new() -> Self {
        Self {
            modifier_state: ModifierKeysState::new(),
        }
    }

    /// Adds the character to the input buffer of the virtual terminal.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/keyboard.c#L675>.
    fn push_input(&self, ch: char, vt: &Tty<VtDriver>) {
        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };
        let mode = vt_console.vt_keyboard().mode();

        if mode == KeyboardMode::Unicode {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let _ = vt.push_input(s.as_bytes());
        } else if ch.is_ascii() {
            let _ = vt.push_input(&[ch as u8]);
        }
    }

    fn handle_letter(&self, ch: char, vt: &Tty<VtDriver>) {
        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };

        let caps_on = vt_console
            .vt_keyboard()
            .lock_keys_state()
            .flags()
            .contains(LockKeyFlags::CAPS_LOCK);

        let out = if caps_on && ch.is_ascii_alphabetic() {
            if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            }
        } else {
            ch
        };

        self.push_input(out, vt);
    }

    fn handle_meta(&self, ch: char, vt: &Tty<VtDriver>) {
        debug_assert!(ch.is_ascii());

        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };
        let mode_flags = vt_console.vt_keyboard().mode_flags();
        if mode_flags.contains(KeyboardModeFlags::META) {
            let _ = vt.push_input(b"\x1b");
            let _ = vt.push_input(&[ch as u8]);
        } else {
            let _ = vt.push_input(&[ch as u8]);
        }
    }

    fn handle_modifier(&self, modifier_key: ModifierKey, is_pressed: bool) {
        match modifier_key {
            ModifierKey::Shift => {
                if is_pressed {
                    self.modifier_state.press(ModifierKeyFlags::SHIFT);
                } else {
                    self.modifier_state.release(ModifierKeyFlags::SHIFT);
                }
            }
            ModifierKey::Ctrl => {
                if is_pressed {
                    self.modifier_state.press(ModifierKeyFlags::CTRL);
                } else {
                    self.modifier_state.release(ModifierKeyFlags::CTRL);
                }
            }
            ModifierKey::Alt => {
                if is_pressed {
                    self.modifier_state.press(ModifierKeyFlags::ALT);
                } else {
                    self.modifier_state.release(ModifierKeyFlags::ALT);
                }
            }
        }
    }

    fn handle_numpad(&self, numpad_key: NumpadKey, vt: &Tty<VtDriver>) {
        let Some(vc) = vt.driver().vt_console() else {
            return;
        };
        let vt_keyboard = vc.vt_keyboard();
        let mode_flags = vt_keyboard.mode_flags();
        let lock_flags = vt_keyboard.lock_keys_state().flags();

        let shift_down = self
            .modifier_state
            .flags()
            .contains(ModifierKeyFlags::SHIFT);
        let num_lock_on = lock_flags.contains(LockKeyFlags::NUM_LOCK);
        let application_mode = mode_flags.contains(KeyboardModeFlags::APPLICATION);

        if application_mode && !shift_down {
            let _ = match numpad_key {
                NumpadKey::Num0 => vt.push_input(b"\x1bOp"),     // p
                NumpadKey::Num1 => vt.push_input(b"\x1bOq"),     // q
                NumpadKey::Num2 => vt.push_input(b"\x1bOr"),     // r
                NumpadKey::Num3 => vt.push_input(b"\x1bOs"),     // s
                NumpadKey::Num4 => vt.push_input(b"\x1bOt"),     // t
                NumpadKey::Num5 => vt.push_input(b"\x1bOu"),     // u
                NumpadKey::Num6 => vt.push_input(b"\x1bOv"),     // v
                NumpadKey::Num7 => vt.push_input(b"\x1bOw"),     // w
                NumpadKey::Num8 => vt.push_input(b"\x1bOx"),     // x
                NumpadKey::Num9 => vt.push_input(b"\x1bOy"),     // y
                NumpadKey::Plus => vt.push_input(b"\x1bOl"),     // l
                NumpadKey::Minus => vt.push_input(b"\x1bOS"),    // S
                NumpadKey::Asterisk => vt.push_input(b"\x1bOR"), // R
                NumpadKey::Slash => vt.push_input(b"\x1bOQ"),    // Q
                NumpadKey::Enter => vt.push_input(b"\x1bOM"),    // M
                NumpadKey::Dot => vt.push_input(b"\x1bOn"),      // n
            };
            return;
        }

        if !num_lock_on {
            match numpad_key {
                NumpadKey::Num0 => self.handle_function(FuncId::Insert, vt),
                NumpadKey::Num1 => self.handle_function(FuncId::Select, vt),
                NumpadKey::Num2 => self.handle_cursor(CursorKey::Down, vt),
                NumpadKey::Num3 => self.handle_function(FuncId::Next, vt),
                NumpadKey::Num4 => self.handle_cursor(CursorKey::Left, vt),
                NumpadKey::Num5 => {
                    if application_mode {
                        let _ = vt.push_input(b"\x1bOG");
                    } else {
                        let _ = vt.push_input(b"\x1b[G");
                    }
                }
                NumpadKey::Num6 => self.handle_cursor(CursorKey::Right, vt),
                NumpadKey::Num7 => self.handle_function(FuncId::Find, vt),
                NumpadKey::Num8 => self.handle_cursor(CursorKey::Up, vt),
                NumpadKey::Num9 => self.handle_function(FuncId::Prior, vt),
                NumpadKey::Dot => self.handle_function(FuncId::Remove, vt),
                _ => {}
            }
            return;
        }

        let ch = match numpad_key {
            NumpadKey::Num0 => '0',
            NumpadKey::Num1 => '1',
            NumpadKey::Num2 => '2',
            NumpadKey::Num3 => '3',
            NumpadKey::Num4 => '4',
            NumpadKey::Num5 => '5',
            NumpadKey::Num6 => '6',
            NumpadKey::Num7 => '7',
            NumpadKey::Num8 => '8',
            NumpadKey::Num9 => '9',
            NumpadKey::Dot => '.',
            NumpadKey::Enter => '\r',
            NumpadKey::Plus => '+',
            NumpadKey::Minus => '-',
            NumpadKey::Asterisk => '*',
            NumpadKey::Slash => '/',
        };

        let _ = vt.push_input(&[ch as u8]);

        if matches!(numpad_key, NumpadKey::Enter) && mode_flags.contains(KeyboardModeFlags::CRLF) {
            let _ = vt.push_input(b"\n");
        }
    }

    fn handle_function(&self, id: FuncId, vt: &Tty<VtDriver>) {
        if let Some(seq) = get_func_bytes(id) {
            let _ = vt.push_input(seq);
        }
    }

    fn handle_cursor(&self, cursor_key: CursorKey, vt: &Tty<VtDriver>) {
        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };
        let cursor_key_mode = vt_console
            .vt_keyboard()
            .mode_flags()
            .contains(KeyboardModeFlags::CURSOR_KEY);

        if cursor_key_mode {
            let _ = match cursor_key {
                CursorKey::Up => vt.push_input(b"\x1bOA"),
                CursorKey::Down => vt.push_input(b"\x1bOB"),
                CursorKey::Left => vt.push_input(b"\x1bOD"),
                CursorKey::Right => vt.push_input(b"\x1bOC"),
            };
        } else {
            let _ = match cursor_key {
                CursorKey::Up => vt.push_input(b"\x1b[A"),
                CursorKey::Down => vt.push_input(b"\x1b[B"),
                CursorKey::Left => vt.push_input(b"\x1b[D"),
                CursorKey::Right => vt.push_input(b"\x1b[C"),
            };
        }
    }

    fn handle_special(&self, handler: SpecialHandler, vt: &Tty<VtDriver>) {
        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };
        let vt_keyboard = vt_console.vt_keyboard();

        match handler {
            SpecialHandler::DecreaseConsole => {
                let vtm = VIRTUAL_TERMINAL_MANAGER
                    .get()
                    .expect("`VIRTUAL_TERMINAL_MANAGER` is not initialized");

                if let Err(e) = vtm.dec_console() {
                    log::warn!("dec_console failed: {:?}", e);
                }
            }
            SpecialHandler::IncreaseConsole => {
                let vtm = VIRTUAL_TERMINAL_MANAGER
                    .get()
                    .expect("`VIRTUAL_TERMINAL_MANAGER` is not initialized");

                if let Err(e) = vtm.inc_console() {
                    log::warn!("inc_console failed: {:?}", e);
                }
            }
            SpecialHandler::ToggleCapsLock => {
                vt_keyboard
                    .lock_keys_state()
                    .toggle(LockKeyFlags::CAPS_LOCK);
            }
            SpecialHandler::ToggleNumLock => {
                let application_mode = vt_keyboard
                    .mode_flags()
                    .contains(KeyboardModeFlags::APPLICATION);
                if application_mode {
                    let _ = vt.push_input(b"\x1bOP");
                } else {
                    vt_keyboard.lock_keys_state().toggle(LockKeyFlags::NUM_LOCK);
                }
            }
            SpecialHandler::ToggleBareNumLock => {
                vt_keyboard.lock_keys_state().toggle(LockKeyFlags::NUM_LOCK);
            }
            SpecialHandler::ToggleScrollLock => {
                // TODO: Scroll lock will affect the scrolling behavior of the console.
                // For now, we just toggle the state.
                vt_keyboard
                    .lock_keys_state()
                    .toggle(LockKeyFlags::SCROLL_LOCK);
            }
            SpecialHandler::Enter => {
                let _ = vt.push_input(b"\r");
                if vt_keyboard.mode_flags().contains(KeyboardModeFlags::CRLF) {
                    let _ = vt.push_input(b"\n");
                }
            }
            SpecialHandler::ScrollBackward
            | SpecialHandler::ScrollForward
            | SpecialHandler::ShowMem
            | SpecialHandler::ShowState
            | SpecialHandler::Compose
            | SpecialHandler::Reboot => {
                log::warn!("VT keyboard action {:?} is not implemented yet", handler);
            }
        }
    }

    /// Handles key events in Medium Raw mode.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/keyboard.c#L1436-L1454>
    fn handle_medium_mode(&self, keycode: KeyCode, key_status: KeyStatus, vt: &Tty<VtDriver>) {
        const UP_FLAG: u8 = 0x80;

        let up_flag = if matches!(key_status, KeyStatus::Pressed) {
            0
        } else {
            UP_FLAG
        };
        let kc: u16 = keycode as u16;

        if kc < 128 {
            let _ = vt.push_input(&[(kc as u8) | up_flag]);
        } else {
            let b0 = up_flag;
            let b1 = ((kc >> 7) as u8) | UP_FLAG;
            let b2 = (kc as u8) | UP_FLAG;
            let _ = vt.push_input(&[b0, b1, b2]);
        }
    }

    fn handle_key_event(&self, keycode: KeyCode, key_status: KeyStatus) {
        log::trace!(
            "VT keyboard handler received key event: keycode={:?}, status={:?}",
            keycode,
            key_status
        );

        let vt = active_vt();
        let Some(vt_console) = vt.driver().vt_console() else {
            return;
        };
        let keyboard_mode = vt_console.vt_keyboard().mode();

        if keyboard_mode == KeyboardMode::MediumRaw {
            self.handle_medium_mode(keycode, key_status, &vt);
        }

        let key_sym = get_keysym(self.modifier_state.flags(), keycode);

        // In Medium Raw, Raw, or Off mode, ignore non-modifier keys
        if (keyboard_mode == KeyboardMode::MediumRaw
            || keyboard_mode == KeyboardMode::Raw
            || keyboard_mode == KeyboardMode::Off)
            && !matches!(key_sym, KeySym::Modifier(_))
        {
            // FIXME: For now, we don't support SAK (secure attention key). And
            // SAK is allowed even in raw mode.
            return;
        }

        // Ignore key release events except for modifier keys
        if key_status == KeyStatus::Released && !matches!(key_sym, KeySym::Modifier(_)) {
            return;
        }

        match key_sym {
            KeySym::Char(ch) => self.push_input(ch, &vt),
            KeySym::Letter(ch) => self.handle_letter(ch, &vt),
            KeySym::Meta(ch) => self.handle_meta(ch, &vt),
            KeySym::Modifier(modifier_key) => {
                self.handle_modifier(modifier_key, key_status == KeyStatus::Pressed)
            }
            KeySym::Numpad(numpad_key) => self.handle_numpad(numpad_key, &vt),
            KeySym::Function(id) => self.handle_function(id, &vt),
            KeySym::Cursor(cursor_key) => self.handle_cursor(cursor_key, &vt),
            KeySym::Special(handler) => self.handle_special(handler, &vt),
            KeySym::SwitchVT(index) => {
                let vtm = VIRTUAL_TERMINAL_MANAGER
                    .get()
                    .expect("`VIRTUAL_TERMINAL_MANAGER` is not initialized");

                if let Err(e) = vtm.switch_vt(index) {
                    log::warn!("switch_vt failed: {:?}", e);
                }
            }
            KeySym::AltNumpad(_) => {
                // TODO: Implement Alt+Numpad input
                log::warn!("Alt+Numpad input is not implemented yet");
            }
            KeySym::NoOp => {
                // Do nothing
            }
        }
    }
}

impl InputHandler for VtKeyboardHandler {
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
                        "Virtual terminal keyboard handler received unsupported event: {:?}",
                        event
                    );
                }
            }
        }
    }
}

pub(super) fn init() {
    let handler_class = Arc::new(VtKeyboardHandlerClass);
    let registered = aster_input::register_handler_class(handler_class);

    core::mem::forget(registered);
}
