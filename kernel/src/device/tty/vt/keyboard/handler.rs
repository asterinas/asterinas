// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::mode::{KeyboardMode, KeyboardModeFlags};
use aster_framebuffer::{ConsoleCallbacks, FRAMEBUFFER_CONSOLE};
use aster_input::{
    event_type_codes::{KeyCode, KeyStatus},
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass, RegisteredInputHandlerClass},
};
use spin::Once;

use crate::device::tty::vt::keyboard::{
    CursorKey, LockKeyFlags, LockKeysState, ModifierKey, ModifierKeyFlags, ModifierKeysState,
    NumpadKey,
    keysym::{FuncId, KeySym, SpecialHandler, get_keysym},
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
        ostd::info!(
            "Virtual terminal keyboard handler connected to device: {}",
            dev.name()
        );
        Ok(Arc::new(VtKeyboardHandler))
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        ostd::info!(
            "Virtual terminal keyboard handler disconnected from device: {}",
            dev.name()
        );
    }
}

/// The state of modifier keys (Shift, Ctrl, Alt).
///
/// It is shared across all virtual terminals.
static MODIFIER_KEYS_STATE: ModifierKeysState = ModifierKeysState::new();

/// The state of lock keys (Caps Lock, Num Lock, Scroll Lock).
///
/// We should have one for each virtual terminal, but now
/// we only have one virtual terminal, so we just use a global state.
static LOCK_KEYS_STATE: LockKeysState = LockKeysState::new();

/// A virtual terminal keyboard handler that handles input from a specific device.
#[derive(Debug)]
struct VtKeyboardHandler;

impl VtKeyboardHandler {
    fn handle_key_event(&self, keycode: KeyCode, key_status: KeyStatus) {
        ostd::debug!(
            "Virtual terminal keyboard handler received key event: keycode={:?}, status={:?}",
            keycode,
            key_status
        );

        let Some(console) = FRAMEBUFFER_CONSOLE.get() else {
            return;
        };

        let console_callbacks = console.lock_callbacks();
        let keyboard_mode = console_callbacks.keyboard_mode();

        if keyboard_mode == KeyboardMode::MediumRaw {
            self.handle_medium_mode(keycode, key_status, &console_callbacks);
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
            KeySym::Char(ch) => self.push_input(ch, &console_callbacks),
            KeySym::Letter(ch) => self.handle_letter(ch, &console_callbacks),
            KeySym::Meta(ch) => self.handle_meta(ch, &console_callbacks),
            KeySym::Modifier(modifier_key) => {
                self.handle_modifier(modifier_key, key_status == KeyStatus::Pressed)
            }
            KeySym::Numpad(numpad_key) => self.handle_numpad(numpad_key, &console_callbacks),
            KeySym::Function(id) => self.handle_function(id, &console_callbacks),
            KeySym::Cursor(cursor_key) => self.handle_cursor(cursor_key, &console_callbacks),
            KeySym::Special(handler) => self.handle_special(handler, &console_callbacks),
            KeySym::SwitchVt(_) => {
                // TODO: Implement virtual terminal switching
                ostd::warn!("Switching virtual terminal is not implemented yet");
            }
            KeySym::AltNumpad(_) => {
                // TODO: Implement Alt+Numpad input
                ostd::warn!("Alt+Numpad input is not implemented yet");
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
    fn handle_medium_mode(
        &self,
        keycode: KeyCode,
        key_status: KeyStatus,
        console_callbacks: &ConsoleCallbacks,
    ) {
        const UP_FLAG: u8 = 0x80;

        let up_bit = if matches!(key_status, KeyStatus::Pressed) {
            0
        } else {
            UP_FLAG
        };
        let kc = keycode as u16;

        if kc < 128 {
            console_callbacks.trigger_callbacks(&[(kc as u8) | up_bit]);
        } else {
            let b0 = up_bit;
            let b1 = ((kc >> 7) as u8) | UP_FLAG;
            let b2 = (kc as u8) | UP_FLAG;
            console_callbacks.trigger_callbacks(&[b0, b1, b2]);
        }
    }

    fn handle_letter(&self, ch: char, console_callbacks: &ConsoleCallbacks) {
        let caps_on = LOCK_KEYS_STATE.flags().contains(LockKeyFlags::CAPS_LOCK);

        let out = if caps_on && ch.is_ascii_alphabetic() {
            if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            }
        } else {
            ch
        };

        self.push_input(out, console_callbacks);
    }

    fn handle_meta(&self, ch: char, console_callbacks: &ConsoleCallbacks) {
        debug_assert!(ch.is_ascii());

        let mode_flags = console_callbacks.keyboard_mode_flags();
        if mode_flags.contains(KeyboardModeFlags::META) {
            console_callbacks.trigger_callbacks(&[0x1b, ch as u8]);
        } else {
            console_callbacks.trigger_callbacks(&[ch as u8]);
        }
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

    fn handle_numpad(&self, numpad_key: NumpadKey, console_callbacks: &ConsoleCallbacks) {
        let mode_flags = console_callbacks.keyboard_mode_flags();

        let shift_down = MODIFIER_KEYS_STATE
            .flags()
            .contains(ModifierKeyFlags::SHIFT);
        let num_lock_on = LOCK_KEYS_STATE.flags().contains(LockKeyFlags::NUM_LOCK);
        let application_mode = mode_flags.contains(KeyboardModeFlags::APPLICATION);

        if application_mode && !shift_down {
            match numpad_key {
                NumpadKey::Num0 => console_callbacks.trigger_callbacks(b"\x1bOp"), // p
                NumpadKey::Num1 => console_callbacks.trigger_callbacks(b"\x1bOq"), // q
                NumpadKey::Num2 => console_callbacks.trigger_callbacks(b"\x1bOr"), // r
                NumpadKey::Num3 => console_callbacks.trigger_callbacks(b"\x1bOs"), // s
                NumpadKey::Num4 => console_callbacks.trigger_callbacks(b"\x1bOt"), // t
                NumpadKey::Num5 => console_callbacks.trigger_callbacks(b"\x1bOu"), // u
                NumpadKey::Num6 => console_callbacks.trigger_callbacks(b"\x1bOv"), // v
                NumpadKey::Num7 => console_callbacks.trigger_callbacks(b"\x1bOw"), // w
                NumpadKey::Num8 => console_callbacks.trigger_callbacks(b"\x1bOx"), // x
                NumpadKey::Num9 => console_callbacks.trigger_callbacks(b"\x1bOy"), // y
                NumpadKey::Plus => console_callbacks.trigger_callbacks(b"\x1bOl"), // l
                NumpadKey::Minus => console_callbacks.trigger_callbacks(b"\x1bOS"), // S
                NumpadKey::Asterisk => console_callbacks.trigger_callbacks(b"\x1bOR"), // R
                NumpadKey::Slash => console_callbacks.trigger_callbacks(b"\x1bOQ"), // Q
                NumpadKey::Enter => console_callbacks.trigger_callbacks(b"\x1bOM"), // M
                NumpadKey::Dot => console_callbacks.trigger_callbacks(b"\x1bOn"),  // n
            }
            return;
        }

        if !num_lock_on {
            match numpad_key {
                NumpadKey::Num0 => self.handle_function(FuncId::Insert, console_callbacks),
                NumpadKey::Num1 => self.handle_function(FuncId::Select, console_callbacks),
                NumpadKey::Num2 => self.handle_cursor(CursorKey::Down, console_callbacks),
                NumpadKey::Num3 => self.handle_function(FuncId::Next, console_callbacks),
                NumpadKey::Num4 => self.handle_cursor(CursorKey::Left, console_callbacks),
                NumpadKey::Num5 => {
                    if application_mode {
                        console_callbacks.trigger_callbacks(b"\x1bOG");
                    } else {
                        console_callbacks.trigger_callbacks(b"\x1b[G");
                    }
                }
                NumpadKey::Num6 => self.handle_cursor(CursorKey::Right, console_callbacks),
                NumpadKey::Num7 => self.handle_function(FuncId::Find, console_callbacks),
                NumpadKey::Num8 => self.handle_cursor(CursorKey::Up, console_callbacks),
                NumpadKey::Num9 => self.handle_function(FuncId::Prior, console_callbacks),
                NumpadKey::Dot => self.handle_function(FuncId::Remove, console_callbacks),
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

        console_callbacks.trigger_callbacks(&[ch as u8]);

        if matches!(numpad_key, NumpadKey::Enter) && mode_flags.contains(KeyboardModeFlags::CRLF) {
            console_callbacks.trigger_callbacks(b"\n");
        }
    }

    fn handle_function(&self, id: FuncId, console_callbacks: &ConsoleCallbacks) {
        if let Some(seq) = id.to_function_string() {
            console_callbacks.trigger_callbacks(seq);
        }
    }

    fn handle_cursor(&self, cursor_key: CursorKey, console_callbacks: &ConsoleCallbacks) {
        let cursor_key_mode = console_callbacks
            .keyboard_mode_flags()
            .contains(KeyboardModeFlags::CURSOR_KEY);

        if cursor_key_mode {
            match cursor_key {
                CursorKey::Up => console_callbacks.trigger_callbacks(b"\x1bOA"),
                CursorKey::Down => console_callbacks.trigger_callbacks(b"\x1bOB"),
                CursorKey::Left => console_callbacks.trigger_callbacks(b"\x1bOD"),
                CursorKey::Right => console_callbacks.trigger_callbacks(b"\x1bOC"),
            }
        } else {
            match cursor_key {
                CursorKey::Up => console_callbacks.trigger_callbacks(b"\x1b[A"),
                CursorKey::Down => console_callbacks.trigger_callbacks(b"\x1b[B"),
                CursorKey::Left => console_callbacks.trigger_callbacks(b"\x1b[D"),
                CursorKey::Right => console_callbacks.trigger_callbacks(b"\x1b[C"),
            }
        }
    }

    fn handle_special(&self, handler: SpecialHandler, console_callbacks: &ConsoleCallbacks) {
        match handler {
            SpecialHandler::ToggleCapsLock => {
                LOCK_KEYS_STATE.toggle(LockKeyFlags::CAPS_LOCK);
            }
            SpecialHandler::ToggleNumLock => {
                let application_mode = console_callbacks
                    .keyboard_mode_flags()
                    .contains(KeyboardModeFlags::APPLICATION);
                if application_mode {
                    console_callbacks.trigger_callbacks(b"\x1bOP");
                } else {
                    LOCK_KEYS_STATE.toggle(LockKeyFlags::NUM_LOCK);
                }
            }
            SpecialHandler::ToggleBareNumLock => {
                LOCK_KEYS_STATE.toggle(LockKeyFlags::NUM_LOCK);
            }
            SpecialHandler::ToggleScrollLock => {
                // TODO: Scroll Lock will affect the scrolling behavior of the console.
                // For now, we just toggle the state.
                LOCK_KEYS_STATE.toggle(LockKeyFlags::SCROLL_LOCK);
            }
            SpecialHandler::Enter => {
                console_callbacks.trigger_callbacks(b"\r");
                if console_callbacks
                    .keyboard_mode_flags()
                    .contains(KeyboardModeFlags::CRLF)
                {
                    console_callbacks.trigger_callbacks(b"\n");
                }
            }
            SpecialHandler::DecreaseConsole
            | SpecialHandler::IncreaseConsole
            | SpecialHandler::ScrollBackward
            | SpecialHandler::ScrollForward
            | SpecialHandler::ShowMem
            | SpecialHandler::ShowState
            | SpecialHandler::Compose
            | SpecialHandler::Reboot => {
                ostd::warn!("VT keyboard action {:?} is not implemented yet", handler);
            }
        }
    }

    /// Adds the character to the input buffer of the virtual terminal.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/keyboard.c#L675>.
    fn push_input(&self, ch: char, console_callbacks: &ConsoleCallbacks) {
        let keyboard_mode = console_callbacks.keyboard_mode();

        if keyboard_mode == KeyboardMode::Unicode {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            console_callbacks.trigger_callbacks(s.as_bytes());
        } else if ch.is_ascii() {
            console_callbacks.trigger_callbacks(&[ch as u8]);
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
                    ostd::warn!(
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
    if FRAMEBUFFER_CONSOLE.get().is_none() {
        return;
    }

    REGISTERED_INPUT_HANDLER_CLASS.call_once(|| {
        let handler_class = Arc::new(VtKeyboardHandlerClass);
        aster_input::register_handler_class(handler_class)
    });
}
