// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::mode::{KeyboardMode, KeyboardModeFlags};
use aster_framebuffer::FRAMEBUFFER;
use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass, RegisteredInputHandlerClass},
};
use spin::Once;

use crate::device::tty::{
    Tty,
    vt::{
        driver::VtDriver,
        keyboard::{
            CursorKey, LockKeyFlags, ModifierKey, ModifierKeyFlags, ModifierKeysState, NumpadKey,
            VtKeyboard,
            keysym::{FuncId, KeySym, SpecialHandler, get_keysym},
        },
        manager::{VT_MANAGER, active_vt},
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
        Ok(Arc::new(VtKeyboardHandler))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        log::info!(
            "Virtual terminal keyboard handler disconnected from device: {}",
            dev.name()
        );
    }
}

/// The state of modifier keys (Shift, Ctrl, Alt).
///
/// It is shared across all virtual terminals.
static MODIFIER_KEYS_STATE: ModifierKeysState = ModifierKeysState::new();

/// A virtual terminal keyboard handler that handles input from a specific device.
#[derive(Debug)]
struct VtKeyboardHandler;

impl VtKeyboardHandler {
    fn handle_key_event(&self, keycode: KeyCode, key_status: KeyStatus) {
        log::trace!(
            "Virtual terminal keyboard handler received key event: keycode={:?}, status={:?}",
            keycode,
            key_status
        );

        let vt = active_vt();
        let vt_console = vt.driver().vt_console();
        let mut vt_keyboard = vt_console.lock_keyboard();
        let keyboard_mode = vt_keyboard.mode();

        if keyboard_mode == KeyboardMode::MediumRaw {
            self.handle_medium_mode(keycode, key_status, vt);
        }

        // Modifier keys are handled based on the keycode instead of the keymap.
        //
        // This is important because the keymap lookup depends on the current
        // modifier state. If a modifier is released while multiple modifiers
        // are active (e.g. Shift+Alt), the keymap may not contain an entry for
        // that modifier combination. In that case the lookup would return
        // `KeySym::NoOp`, and the release event would be ignored, causing the
        // modifier state to become stuck.
        //
        // By detecting modifiers directly from the keycode, we ensure that
        // modifier press/release events are always processed correctly,
        // regardless of the current modifier combination.
        let key_sym = if let Some(modifier) = self.keycode_to_modifier_key(keycode) {
            KeySym::Modifier(modifier)
        } else {
            get_keysym(MODIFIER_KEYS_STATE.flags(), keycode)
        };

        // In Medium Raw, Raw, or Off mode, ignore non-modifier keys
        if (keyboard_mode == KeyboardMode::MediumRaw
            || keyboard_mode == KeyboardMode::Raw
            || keyboard_mode == KeyboardMode::Off)
            && !matches!(key_sym, KeySym::Modifier(_))
        {
            // FIXME: For now, we don't support SAK (secure attention key).
            // SAK is allowed even in Raw mode.
            return;
        }

        // Ignore key release events except for modifier keys
        if key_status == KeyStatus::Released && !matches!(key_sym, KeySym::Modifier(_)) {
            return;
        }

        match key_sym {
            KeySym::Char(ch) => self.push_input(ch, vt, &vt_keyboard),
            KeySym::Letter(ch) => self.handle_letter(ch, vt, &vt_keyboard),
            KeySym::Meta(ch) => self.handle_meta(ch, vt, &vt_keyboard),
            KeySym::Modifier(modifier_key) => {
                self.handle_modifier(modifier_key, key_status == KeyStatus::Pressed)
            }
            KeySym::Numpad(numpad_key) => self.handle_numpad(numpad_key, vt, &vt_keyboard),
            KeySym::Function(id) => self.handle_function(id, vt),
            KeySym::Cursor(cursor_key) => self.handle_cursor(cursor_key, vt, &vt_keyboard),
            KeySym::Special(handler) => self.handle_special(handler, vt, &mut vt_keyboard),
            KeySym::SwitchVt(index) => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if let Err(e) = vtm.with_lock(|m| m.switch_vt(index)) {
                    log::warn!("VT switching failed: {:?}", e);
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

    fn keycode_to_modifier_key(&self, keycode: KeyCode) -> Option<ModifierKey> {
        match keycode {
            KeyCode::LeftShift | KeyCode::RightShift => Some(ModifierKey::Shift),
            KeyCode::LeftCtrl | KeyCode::RightCtrl => Some(ModifierKey::Ctrl),
            KeyCode::LeftAlt | KeyCode::RightAlt => Some(ModifierKey::Alt),
            _ => None,
        }
    }

    /// Handles key events in Medium Raw mode.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/keyboard.c#L1436-L1454>
    fn handle_medium_mode(&self, keycode: KeyCode, key_status: KeyStatus, vt: &Tty<VtDriver>) {
        const UP_FLAG: u8 = 0x80;

        let up_bit = if matches!(key_status, KeyStatus::Pressed) {
            0
        } else {
            UP_FLAG
        };
        let kc = keycode as u16;

        let _ = if kc < 128 {
            vt.push_input(&[(kc as u8) | up_bit])
        } else {
            let b0 = up_bit;
            let b1 = ((kc >> 7) as u8) | UP_FLAG;
            let b2 = (kc as u8) | UP_FLAG;
            vt.push_input(&[b0, b1, b2])
        };
    }

    fn handle_letter(&self, ch: char, vt: &Tty<VtDriver>, vt_keyboard: &VtKeyboard) {
        let caps_on = vt_keyboard
            .lock_key_flags()
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

        self.push_input(out, vt, vt_keyboard);
    }

    fn handle_meta(&self, ch: char, vt: &Tty<VtDriver>, vt_keyboard: &VtKeyboard) {
        debug_assert!(ch.is_ascii());

        let mode_flags = vt_keyboard.mode_flags();
        let _ = if mode_flags.contains(KeyboardModeFlags::META) {
            vt.push_input(&[0x1b, ch as u8])
        } else {
            vt.push_input(&[ch as u8])
        };
    }

    fn handle_modifier(&self, modifier_key: ModifierKey, is_pressed: bool) {
        match modifier_key {
            ModifierKey::Shift => {
                if is_pressed {
                    MODIFIER_KEYS_STATE.press(ModifierKeyFlags::SHIFT);
                } else {
                    MODIFIER_KEYS_STATE.release(ModifierKeyFlags::SHIFT);
                }
            }
            ModifierKey::Ctrl => {
                if is_pressed {
                    MODIFIER_KEYS_STATE.press(ModifierKeyFlags::CTRL);
                } else {
                    MODIFIER_KEYS_STATE.release(ModifierKeyFlags::CTRL);
                }
            }
            ModifierKey::Alt => {
                if is_pressed {
                    MODIFIER_KEYS_STATE.press(ModifierKeyFlags::ALT);
                } else {
                    MODIFIER_KEYS_STATE.release(ModifierKeyFlags::ALT);
                }
            }
        }
    }

    fn handle_numpad(&self, numpad_key: NumpadKey, vt: &Tty<VtDriver>, vt_keyboard: &VtKeyboard) {
        let mode_flags = vt_keyboard.mode_flags();

        let shift_down = MODIFIER_KEYS_STATE
            .flags()
            .contains(ModifierKeyFlags::SHIFT);
        let num_lock_on = vt_keyboard
            .lock_key_flags()
            .contains(LockKeyFlags::NUM_LOCK);
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
                NumpadKey::Num2 => self.handle_cursor(CursorKey::Down, vt, vt_keyboard),
                NumpadKey::Num3 => self.handle_function(FuncId::Next, vt),
                NumpadKey::Num4 => self.handle_cursor(CursorKey::Left, vt, vt_keyboard),
                NumpadKey::Num5 => {
                    let _ = if application_mode {
                        vt.push_input(b"\x1bOG")
                    } else {
                        vt.push_input(b"\x1b[G")
                    };
                }
                NumpadKey::Num6 => self.handle_cursor(CursorKey::Right, vt, vt_keyboard),
                NumpadKey::Num7 => self.handle_function(FuncId::Find, vt),
                NumpadKey::Num8 => self.handle_cursor(CursorKey::Up, vt, vt_keyboard),
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
        if let Some(seq) = id.to_function_string() {
            let _ = vt.push_input(seq);
        }
    }

    fn handle_cursor(&self, cursor_key: CursorKey, vt: &Tty<VtDriver>, vt_keyboard: &VtKeyboard) {
        let cursor_key_mode = vt_keyboard
            .mode_flags()
            .contains(KeyboardModeFlags::CURSOR_KEY);

        let _ = if cursor_key_mode {
            match cursor_key {
                CursorKey::Up => vt.push_input(b"\x1bOA"),
                CursorKey::Down => vt.push_input(b"\x1bOB"),
                CursorKey::Left => vt.push_input(b"\x1bOD"),
                CursorKey::Right => vt.push_input(b"\x1bOC"),
            }
        } else {
            match cursor_key {
                CursorKey::Up => vt.push_input(b"\x1b[A"),
                CursorKey::Down => vt.push_input(b"\x1b[B"),
                CursorKey::Left => vt.push_input(b"\x1b[D"),
                CursorKey::Right => vt.push_input(b"\x1b[C"),
            }
        };
    }

    fn handle_special(
        &self,
        handler: SpecialHandler,
        vt: &Tty<VtDriver>,
        vt_keyboard: &mut VtKeyboard,
    ) {
        match handler {
            SpecialHandler::ToggleCapsLock => {
                vt_keyboard.toggle_lock_keys(LockKeyFlags::CAPS_LOCK);
            }
            SpecialHandler::ToggleNumLock => {
                let application_mode = vt_keyboard
                    .mode_flags()
                    .contains(KeyboardModeFlags::APPLICATION);
                if application_mode {
                    let _ = vt.push_input(b"\x1bOP");
                } else {
                    let _ = vt.push_input(b"\x1b[O");
                    vt_keyboard.toggle_lock_keys(LockKeyFlags::NUM_LOCK);
                }
            }
            SpecialHandler::ToggleBareNumLock => {
                vt_keyboard.toggle_lock_keys(LockKeyFlags::NUM_LOCK);
            }
            SpecialHandler::ToggleScrollLock => {
                // TODO: Scroll Lock will affect the scrolling behavior of the console.
                // For now, we just toggle the state.
                vt_keyboard.toggle_lock_keys(LockKeyFlags::SCROLL_LOCK);
            }
            SpecialHandler::Enter => {
                let _ = vt.push_input(b"\r");
                if vt_keyboard.mode_flags().contains(KeyboardModeFlags::CRLF) {
                    let _ = vt.push_input(b"\n");
                }
            }
            SpecialHandler::DecreaseConsole => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if let Err(e) = vtm.with_lock(|m| m.dec_console()) {
                    log::warn!("VT decreasing failed: {:?}", e);
                }
            }
            SpecialHandler::IncreaseConsole => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if let Err(e) = vtm.with_lock(|m| m.inc_console()) {
                    log::warn!("VT increasing failed: {:?}", e);
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

    /// Adds the character to the input buffer of the virtual terminal.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/keyboard.c#L675>.
    fn push_input(&self, ch: char, vt: &Tty<VtDriver>, vt_keyboard: &VtKeyboard) {
        let keyboard_mode = vt_keyboard.mode();

        if keyboard_mode == KeyboardMode::Unicode {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let _ = vt.push_input(s.as_bytes());
        } else if ch.is_ascii() {
            let _ = vt.push_input(&[ch as u8]);
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
                    // Nothing to do
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

static REGISTERED_INPUT_HANDLER_CLASS: Once<RegisteredInputHandlerClass> = Once::new();

pub(super) fn init_in_first_process() {
    if FRAMEBUFFER.get().is_none() {
        return;
    }

    REGISTERED_INPUT_HANDLER_CLASS.call_once(|| {
        let handler_class = Arc::new(VtKeyboardHandlerClass);
        aster_input::register_handler_class(handler_class)
    });
}
