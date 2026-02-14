// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use aster_input::event_type_codes::KeyCode;
use spin::Once;

use crate::device::tty::vt::{
    VtIndex,
    keyboard::{CursorKey, ModifierKey, ModifierKeyFlags, NumpadKey},
};

/// The symbolic representation of a key under a given modifier state.
///
// Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/drivers/tty/vt/keyboard.c#L69-L77>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::device::tty::vt::keyboard) enum KeySym {
    /// A direct character output (similar to Linux `k_self`).
    ///
    /// It is used for characters that do not depend on Caps Lock state
    /// (digits, punctuation, etc.).
    Char(char),
    /// An alphabetic character whose case may be affected by Caps Lock
    /// (similar to Linux `k_lowercase`).
    Letter(char),
    /// Meta (usually Alt) + key (similar to Linux `k_meta`).
    ///
    /// The handler may emit an ESC-prefixed sequence depending on the VT keyboard mode flags.
    Meta(char),
    /// A pure modifier key (similar to Linux `k_shift`).
    Modifier(ModifierKey),
    /// A numpad key (similar to Linux `k_pad`).
    Numpad(NumpadKey),
    /// Emit a function-string identified by `FuncId` (similar to Linux `k_fn`).
    Function(FuncId),
    /// Alt+Numpad digit used for ASCII code input (similar to Linux `k_ascii`).
    ///
    /// The handler is responsible for accumulating digits while Alt is held and emitting the
    /// final character when the sequence is committed (typically on Alt release).
    AltNumpad(u8),
    /// A cursor key (similar to Linux `k_cur`).
    Cursor(CursorKey),
    /// A special action (similar to Linux `k_spec`).
    Special(SpecialHandler),
    /// Switch to another virtual terminal (similar to Linux `k_cons`).
    SwitchVT(VtIndex),
    /// No operation.
    NoOp,
}

/// Special key actions that are handled by the VT keyboard layer.
///
// Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/drivers/tty/vt/keyboard.c#L84-L89>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::device::tty::vt::keyboard) enum SpecialHandler {
    /// Toggle Caps Lock state (similar to Linux `fn_caps_toggle`).
    ToggleCapsLock,
    /// Toggle Num Lock state, possibly with special handling in application-keypad mode (similar to Linux `fn_num`).
    ToggleNumLock,
    /// Toggle Num Lock state without any application-keypad side effects (similar to Linux `fn_bare_num`).
    ToggleBareNumLock,
    /// Toggle Scroll Lock state (similar to Linux `fn_hold`).
    ToggleScrollLock,
    // Similar to Linux `fn_show_mem`
    ShowMem,
    // Similar to Linux `fn_show_state`
    ShowState,
    /// Start a compose sequence (similar to Linux `fn_compose`).
    Compose,
    /// Trigger a reboot of the system (similar to Linux `fn_boot_it`).
    Reboot,
    /// Scroll the console history backward (similar to Linux `fn_scroll_back`).
    ScrollBackward,
    /// Scroll the console history forward (similar to Linux `fn_scroll_forward`).
    ScrollForward,
    /// Switch to the previous VT (similar to Linux `fn_dec_console`).
    DecreaseConsole,
    /// Switch to the next VT (similar to Linux `fn_inc_console`).
    IncreaseConsole,
    /// Handle the Enter key (similar to Linux `fn_enter`).
    Enter,
}

/// The key mapping tables for different modifier key combinations.
struct KeyMaps {
    maps: Vec<Vec<KeySym>>,
}

impl Default for KeyMaps {
    /// Create a default keymaps with standard key mappings.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/defkeymap.map> and
    /// <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/defkeymap.c_shipped>
    fn default() -> Self {
        let mut key_maps = Self::new(Self::MOD_COMBINATIONS, Self::KEY_COUNT);
        key_maps.map_letter();
        key_maps.map_char();
        key_maps.map_function();
        key_maps.map_switch_vt();
        key_maps.map_meta();
        key_maps.map_numpad_keys();
        key_maps.map_enter();
        key_maps.map_compose();
        key_maps.map_modifier();
        key_maps.map_reboot();
        key_maps.map_scroll();
        key_maps.map_cursor_keys();
        key_maps.map_lock_keys();
        key_maps
    }
}

type KeyBind = (KeyCode, KeySym);

impl KeyMaps {
    /// Total number of keycodes handled by the default VT keymap.
    ///
    /// Note:
    /// - This table indexes by the numeric value of `KeyCode`.
    /// - Key codes outside this range will map to `KeySym::NoOp`.
    const KEY_COUNT: usize = 128;

    /// Number of modifier combinations supported by the default keymap.
    ///
    /// We index keymaps by the raw 3-bit modifier mask (SHIFT|ALT|CTRL),
    /// thus the table has 2^3 = 8 layers.
    const MOD_COMBINATIONS: usize = 8;

    fn new(mods_num: usize, keycode_num: usize) -> Self {
        Self {
            maps: vec![vec![KeySym::NoOp; keycode_num]; mods_num],
        }
    }

    fn get_keysym(&self, mods: ModifierKeyFlags, key_code: KeyCode) -> KeySym {
        let mods_index = mods.bits() as usize;
        let key_index = key_code as usize;
        if mods_index >= self.maps.len() || key_index >= self.maps[mods_index].len() {
            return KeySym::NoOp;
        }
        self.maps[mods_index][key_index]
    }

    fn set_keysym(&mut self, mods: ModifierKeyFlags, key_code: KeyCode, key_sym: KeySym) {
        let mods_index = mods.bits() as usize;
        let key_index = key_code as usize;
        if mods_index >= self.maps.len() || key_index >= self.maps[mods_index].len() {
            return;
        }
        self.maps[mods_index][key_index] = key_sym;
    }

    fn apply_key_binds(&mut self, mods_list: &[ModifierKeyFlags], binds: &[KeyBind]) {
        for &mods in mods_list {
            for &(key_code, key_sym) in binds {
                self.set_keysym(mods, key_code, key_sym);
            }
        }
    }

    fn map_letter(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::empty()],
            &[
                (KeyCode::A, KeySym::Letter('a')),
                (KeyCode::B, KeySym::Letter('b')),
                (KeyCode::C, KeySym::Letter('c')),
                (KeyCode::D, KeySym::Letter('d')),
                (KeyCode::E, KeySym::Letter('e')),
                (KeyCode::F, KeySym::Letter('f')),
                (KeyCode::G, KeySym::Letter('g')),
                (KeyCode::H, KeySym::Letter('h')),
                (KeyCode::I, KeySym::Letter('i')),
                (KeyCode::J, KeySym::Letter('j')),
                (KeyCode::K, KeySym::Letter('k')),
                (KeyCode::L, KeySym::Letter('l')),
                (KeyCode::M, KeySym::Letter('m')),
                (KeyCode::N, KeySym::Letter('n')),
                (KeyCode::O, KeySym::Letter('o')),
                (KeyCode::P, KeySym::Letter('p')),
                (KeyCode::Q, KeySym::Letter('q')),
                (KeyCode::R, KeySym::Letter('r')),
                (KeyCode::S, KeySym::Letter('s')),
                (KeyCode::T, KeySym::Letter('t')),
                (KeyCode::U, KeySym::Letter('u')),
                (KeyCode::V, KeySym::Letter('v')),
                (KeyCode::W, KeySym::Letter('w')),
                (KeyCode::X, KeySym::Letter('x')),
                (KeyCode::Y, KeySym::Letter('y')),
                (KeyCode::Z, KeySym::Letter('z')),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[
                (KeyCode::A, KeySym::Letter('A')),
                (KeyCode::B, KeySym::Letter('B')),
                (KeyCode::C, KeySym::Letter('C')),
                (KeyCode::D, KeySym::Letter('D')),
                (KeyCode::E, KeySym::Letter('E')),
                (KeyCode::F, KeySym::Letter('F')),
                (KeyCode::G, KeySym::Letter('G')),
                (KeyCode::H, KeySym::Letter('H')),
                (KeyCode::I, KeySym::Letter('I')),
                (KeyCode::J, KeySym::Letter('J')),
                (KeyCode::K, KeySym::Letter('K')),
                (KeyCode::L, KeySym::Letter('L')),
                (KeyCode::M, KeySym::Letter('M')),
                (KeyCode::N, KeySym::Letter('N')),
                (KeyCode::O, KeySym::Letter('O')),
                (KeyCode::P, KeySym::Letter('P')),
                (KeyCode::Q, KeySym::Letter('Q')),
                (KeyCode::R, KeySym::Letter('R')),
                (KeyCode::S, KeySym::Letter('S')),
                (KeyCode::T, KeySym::Letter('T')),
                (KeyCode::U, KeySym::Letter('U')),
                (KeyCode::V, KeySym::Letter('V')),
                (KeyCode::W, KeySym::Letter('W')),
                (KeyCode::X, KeySym::Letter('X')),
                (KeyCode::Y, KeySym::Letter('Y')),
                (KeyCode::Z, KeySym::Letter('Z')),
            ],
        );
    }

    fn map_char(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::empty()],
            &[
                (KeyCode::Esc, KeySym::Char('\x1b')),
                (KeyCode::Num1, KeySym::Char('1')),
                (KeyCode::Num2, KeySym::Char('2')),
                (KeyCode::Num3, KeySym::Char('3')),
                (KeyCode::Num4, KeySym::Char('4')),
                (KeyCode::Num5, KeySym::Char('5')),
                (KeyCode::Num6, KeySym::Char('6')),
                (KeyCode::Num7, KeySym::Char('7')),
                (KeyCode::Num8, KeySym::Char('8')),
                (KeyCode::Num9, KeySym::Char('9')),
                (KeyCode::Num0, KeySym::Char('0')),
                (KeyCode::Minus, KeySym::Char('-')),
                (KeyCode::Equal, KeySym::Char('=')),
                (KeyCode::Backspace, KeySym::Char('\x7f')),
                (KeyCode::Tab, KeySym::Char('\t')),
                (KeyCode::LeftBrace, KeySym::Char('[')),
                (KeyCode::RightBrace, KeySym::Char(']')),
                (KeyCode::Semicolon, KeySym::Char(';')),
                (KeyCode::Apostrophe, KeySym::Char('\'')),
                (KeyCode::Grave, KeySym::Char('`')),
                (KeyCode::Backslash, KeySym::Char('\\')),
                (KeyCode::Comma, KeySym::Char(',')),
                (KeyCode::Dot, KeySym::Char('.')),
                (KeyCode::Slash, KeySym::Char('/')),
                (KeyCode::Space, KeySym::Char(' ')),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[
                (KeyCode::Esc, KeySym::Char('\x1b')),
                (KeyCode::Num1, KeySym::Char('!')),
                (KeyCode::Num2, KeySym::Char('@')),
                (KeyCode::Num3, KeySym::Char('#')),
                (KeyCode::Num4, KeySym::Char('$')),
                (KeyCode::Num5, KeySym::Char('%')),
                (KeyCode::Num6, KeySym::Char('^')),
                (KeyCode::Num7, KeySym::Char('&')),
                (KeyCode::Num8, KeySym::Char('*')),
                (KeyCode::Num9, KeySym::Char('(')),
                (KeyCode::Num0, KeySym::Char(')')),
                (KeyCode::Minus, KeySym::Char('_')),
                (KeyCode::Equal, KeySym::Char('+')),
                (KeyCode::Backspace, KeySym::Char('\x7f')),
                (KeyCode::Tab, KeySym::Char('\t')),
                (KeyCode::LeftBrace, KeySym::Char('{')),
                (KeyCode::RightBrace, KeySym::Char('}')),
                (KeyCode::Semicolon, KeySym::Char(':')),
                (KeyCode::Apostrophe, KeySym::Char('"')),
                (KeyCode::Grave, KeySym::Char('~')),
                (KeyCode::Backslash, KeySym::Char('|')),
                (KeyCode::Comma, KeySym::Char('<')),
                (KeyCode::Dot, KeySym::Char('>')),
                (KeyCode::Slash, KeySym::Char('?')),
                (KeyCode::Space, KeySym::Char(' ')),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::CTRL],
            &[
                (KeyCode::Num2, KeySym::Char('\x00')),
                (KeyCode::Num3, KeySym::Char('\x1b')),
                (KeyCode::Num4, KeySym::Char('\x1c')),
                (KeyCode::Num5, KeySym::Char('\x1d')),
                (KeyCode::Num6, KeySym::Char('\x1e')),
                (KeyCode::Num7, KeySym::Char('\x1f')),
                (KeyCode::Num8, KeySym::Char('\x7f')),
                (KeyCode::Minus, KeySym::Char('\x1f')),
                (KeyCode::Backspace, KeySym::Char('\x08')),
                (KeyCode::A, KeySym::Char('\x01')),
                (KeyCode::B, KeySym::Char('\x02')),
                (KeyCode::C, KeySym::Char('\x03')),
                (KeyCode::D, KeySym::Char('\x04')),
                (KeyCode::E, KeySym::Char('\x05')),
                (KeyCode::F, KeySym::Char('\x06')),
                (KeyCode::G, KeySym::Char('\x07')),
                (KeyCode::H, KeySym::Char('\x08')),
                (KeyCode::I, KeySym::Char('\x09')),
                (KeyCode::J, KeySym::Char('\x0a')),
                (KeyCode::K, KeySym::Char('\x0b')),
                (KeyCode::L, KeySym::Char('\x0c')),
                (KeyCode::M, KeySym::Char('\x0d')),
                (KeyCode::N, KeySym::Char('\x0e')),
                (KeyCode::O, KeySym::Char('\x0f')),
                (KeyCode::P, KeySym::Char('\x10')),
                (KeyCode::Q, KeySym::Char('\x11')),
                (KeyCode::R, KeySym::Char('\x12')),
                (KeyCode::S, KeySym::Char('\x13')),
                (KeyCode::T, KeySym::Char('\x14')),
                (KeyCode::U, KeySym::Char('\x15')),
                (KeyCode::V, KeySym::Char('\x16')),
                (KeyCode::W, KeySym::Char('\x17')),
                (KeyCode::X, KeySym::Char('\x18')),
                (KeyCode::Y, KeySym::Char('\x19')),
                (KeyCode::Z, KeySym::Char('\x1a')),
                (KeyCode::LeftBrace, KeySym::Char('\x1b')),
                (KeyCode::RightBrace, KeySym::Char('\x1d')),
                (KeyCode::Apostrophe, KeySym::Char('\x07')),
                (KeyCode::Grave, KeySym::Char('\x00')),
                (KeyCode::Backslash, KeySym::Char('\x1c')),
                (KeyCode::Slash, KeySym::Char('\x7f')),
                (KeyCode::Space, KeySym::Char('\x00')),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL],
            &[
                (KeyCode::Num2, KeySym::Char('\x00')),
                (KeyCode::Minus, KeySym::Char('\x1f')),
                (KeyCode::A, KeySym::Char('\x01')),
                (KeyCode::B, KeySym::Char('\x02')),
                (KeyCode::C, KeySym::Char('\x03')),
                (KeyCode::D, KeySym::Char('\x04')),
                (KeyCode::E, KeySym::Char('\x05')),
                (KeyCode::F, KeySym::Char('\x06')),
                (KeyCode::G, KeySym::Char('\x07')),
                (KeyCode::H, KeySym::Char('\x08')),
                (KeyCode::I, KeySym::Char('\x09')),
                (KeyCode::J, KeySym::Char('\x0a')),
                (KeyCode::K, KeySym::Char('\x0b')),
                (KeyCode::L, KeySym::Char('\x0c')),
                (KeyCode::M, KeySym::Char('\x0d')),
                (KeyCode::N, KeySym::Char('\x0e')),
                (KeyCode::O, KeySym::Char('\x0f')),
                (KeyCode::P, KeySym::Char('\x10')),
                (KeyCode::Q, KeySym::Char('\x11')),
                (KeyCode::R, KeySym::Char('\x12')),
                (KeyCode::S, KeySym::Char('\x13')),
                (KeyCode::T, KeySym::Char('\x14')),
                (KeyCode::U, KeySym::Char('\x15')),
                (KeyCode::V, KeySym::Char('\x16')),
                (KeyCode::W, KeySym::Char('\x17')),
                (KeyCode::X, KeySym::Char('\x18')),
                (KeyCode::Y, KeySym::Char('\x19')),
                (KeyCode::Z, KeySym::Char('\x1a')),
            ],
        );
    }

    fn map_function(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
            ],
            &[
                (KeyCode::F1, KeySym::Function(FuncId::F1)),
                (KeyCode::F2, KeySym::Function(FuncId::F2)),
                (KeyCode::F3, KeySym::Function(FuncId::F3)),
                (KeyCode::F4, KeySym::Function(FuncId::F4)),
                (KeyCode::F5, KeySym::Function(FuncId::F5)),
                (KeyCode::F6, KeySym::Function(FuncId::F6)),
                (KeyCode::F7, KeySym::Function(FuncId::F7)),
                (KeyCode::F8, KeySym::Function(FuncId::F8)),
                (KeyCode::F9, KeySym::Function(FuncId::F9)),
                (KeyCode::F10, KeySym::Function(FuncId::F10)),
                (KeyCode::F11, KeySym::Function(FuncId::F11)),
                (KeyCode::F12, KeySym::Function(FuncId::F12)),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[
                (KeyCode::F1, KeySym::Function(FuncId::F11)),
                (KeyCode::F2, KeySym::Function(FuncId::F12)),
                (KeyCode::F3, KeySym::Function(FuncId::F13)),
                (KeyCode::F4, KeySym::Function(FuncId::F14)),
                (KeyCode::F5, KeySym::Function(FuncId::F15)),
                (KeyCode::F6, KeySym::Function(FuncId::F16)),
                (KeyCode::F7, KeySym::Function(FuncId::F17)),
                (KeyCode::F8, KeySym::Function(FuncId::F18)),
                (KeyCode::F9, KeySym::Function(FuncId::F19)),
                (KeyCode::F10, KeySym::Function(FuncId::F20)),
            ],
        );

        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::Home, KeySym::Function(FuncId::Find)),
                (KeyCode::End, KeySym::Function(FuncId::Select)),
                (KeyCode::Insert, KeySym::Function(FuncId::Insert)),
                (KeyCode::Delete, KeySym::Function(FuncId::Remove)),
                (KeyCode::Mute, KeySym::Function(FuncId::F13)),
                (KeyCode::VolumeDown, KeySym::Function(FuncId::F14)),
                (KeyCode::VolumeUp, KeySym::Function(FuncId::Help)),
                (KeyCode::Power, KeySym::Function(FuncId::Do)),
                (KeyCode::Pause, KeySym::Function(FuncId::Pause)),
            ],
        );

        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::PageUp, KeySym::Function(FuncId::Prior)),
                (KeyCode::PageDown, KeySym::Function(FuncId::Next)),
            ],
        );
    }

    fn map_switch_vt(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::F1, KeySym::SwitchVT(VtIndex::new(1).unwrap())),
                (KeyCode::F2, KeySym::SwitchVT(VtIndex::new(2).unwrap())),
                (KeyCode::F3, KeySym::SwitchVT(VtIndex::new(3).unwrap())),
                (KeyCode::F4, KeySym::SwitchVT(VtIndex::new(4).unwrap())),
                (KeyCode::F5, KeySym::SwitchVT(VtIndex::new(5).unwrap())),
                (KeyCode::F6, KeySym::SwitchVT(VtIndex::new(6).unwrap())),
                (KeyCode::F7, KeySym::SwitchVT(VtIndex::new(7).unwrap())),
                (KeyCode::F8, KeySym::SwitchVT(VtIndex::new(8).unwrap())),
                (KeyCode::F9, KeySym::SwitchVT(VtIndex::new(9).unwrap())),
                (KeyCode::F10, KeySym::SwitchVT(VtIndex::new(10).unwrap())),
                (KeyCode::F11, KeySym::SwitchVT(VtIndex::new(11).unwrap())),
                (KeyCode::F12, KeySym::SwitchVT(VtIndex::new(12).unwrap())),
            ],
        );
    }

    fn map_meta(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::ALT],
            &[
                (KeyCode::Esc, KeySym::Meta('\x1b')),
                (KeyCode::Num1, KeySym::Meta('1')),
                (KeyCode::Num2, KeySym::Meta('2')),
                (KeyCode::Num3, KeySym::Meta('3')),
                (KeyCode::Num4, KeySym::Meta('4')),
                (KeyCode::Num5, KeySym::Meta('5')),
                (KeyCode::Num6, KeySym::Meta('6')),
                (KeyCode::Num7, KeySym::Meta('7')),
                (KeyCode::Num8, KeySym::Meta('8')),
                (KeyCode::Num9, KeySym::Meta('9')),
                (KeyCode::Num0, KeySym::Meta('0')),
                (KeyCode::Minus, KeySym::Meta('-')),
                (KeyCode::Equal, KeySym::Meta('=')),
                (KeyCode::Backspace, KeySym::Meta('\x7f')),
                (KeyCode::Tab, KeySym::Meta('\t')),
                (KeyCode::A, KeySym::Meta('a')),
                (KeyCode::B, KeySym::Meta('b')),
                (KeyCode::C, KeySym::Meta('c')),
                (KeyCode::D, KeySym::Meta('d')),
                (KeyCode::E, KeySym::Meta('e')),
                (KeyCode::F, KeySym::Meta('f')),
                (KeyCode::G, KeySym::Meta('g')),
                (KeyCode::H, KeySym::Meta('h')),
                (KeyCode::I, KeySym::Meta('i')),
                (KeyCode::J, KeySym::Meta('j')),
                (KeyCode::K, KeySym::Meta('k')),
                (KeyCode::L, KeySym::Meta('l')),
                (KeyCode::M, KeySym::Meta('m')),
                (KeyCode::N, KeySym::Meta('n')),
                (KeyCode::O, KeySym::Meta('o')),
                (KeyCode::P, KeySym::Meta('p')),
                (KeyCode::Q, KeySym::Meta('q')),
                (KeyCode::R, KeySym::Meta('r')),
                (KeyCode::S, KeySym::Meta('s')),
                (KeyCode::T, KeySym::Meta('t')),
                (KeyCode::U, KeySym::Meta('u')),
                (KeyCode::V, KeySym::Meta('v')),
                (KeyCode::W, KeySym::Meta('w')),
                (KeyCode::X, KeySym::Meta('x')),
                (KeyCode::Y, KeySym::Meta('y')),
                (KeyCode::Z, KeySym::Meta('z')),
                (KeyCode::LeftBrace, KeySym::Meta('[')),
                (KeyCode::RightBrace, KeySym::Meta(']')),
                (KeyCode::Enter, KeySym::Meta('\x0d')),
                (KeyCode::Semicolon, KeySym::Meta(';')),
                (KeyCode::Apostrophe, KeySym::Meta('\'')),
                (KeyCode::Grave, KeySym::Meta('`')),
                (KeyCode::Backslash, KeySym::Meta('\\')),
                (KeyCode::Comma, KeySym::Meta(',')),
                (KeyCode::Dot, KeySym::Meta('.')),
                (KeyCode::Slash, KeySym::Meta('/')),
                (KeyCode::Space, KeySym::Meta(' ')),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT],
            &[
                (KeyCode::A, KeySym::Meta('\x01')),
                (KeyCode::B, KeySym::Meta('\x02')),
                (KeyCode::C, KeySym::Meta('\x03')),
                (KeyCode::D, KeySym::Meta('\x04')),
                (KeyCode::E, KeySym::Meta('\x05')),
                (KeyCode::F, KeySym::Meta('\x06')),
                (KeyCode::G, KeySym::Meta('\x07')),
                (KeyCode::H, KeySym::Meta('\x08')),
                (KeyCode::I, KeySym::Meta('\x09')),
                (KeyCode::J, KeySym::Meta('\x0a')),
                (KeyCode::K, KeySym::Meta('\x0b')),
                (KeyCode::L, KeySym::Meta('\x0c')),
                (KeyCode::M, KeySym::Meta('\x0d')),
                (KeyCode::N, KeySym::Meta('\x0e')),
                (KeyCode::O, KeySym::Meta('\x0f')),
                (KeyCode::P, KeySym::Meta('\x10')),
                (KeyCode::Q, KeySym::Meta('\x11')),
                (KeyCode::R, KeySym::Meta('\x12')),
                (KeyCode::S, KeySym::Meta('\x13')),
                (KeyCode::T, KeySym::Meta('\x14')),
                (KeyCode::U, KeySym::Meta('\x15')),
                (KeyCode::V, KeySym::Meta('\x16')),
                (KeyCode::W, KeySym::Meta('\x17')),
                (KeyCode::X, KeySym::Meta('\x18')),
                (KeyCode::Y, KeySym::Meta('\x19')),
                (KeyCode::Z, KeySym::Meta('\x1a')),
            ],
        );
    }

    fn map_numpad_keys(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::Kp0, KeySym::Numpad(NumpadKey::Num0)),
                (KeyCode::Kp1, KeySym::Numpad(NumpadKey::Num1)),
                (KeyCode::Kp2, KeySym::Numpad(NumpadKey::Num2)),
                (KeyCode::Kp3, KeySym::Numpad(NumpadKey::Num3)),
                (KeyCode::Kp4, KeySym::Numpad(NumpadKey::Num4)),
                (KeyCode::Kp5, KeySym::Numpad(NumpadKey::Num5)),
                (KeyCode::Kp6, KeySym::Numpad(NumpadKey::Num6)),
                (KeyCode::Kp7, KeySym::Numpad(NumpadKey::Num7)),
                (KeyCode::Kp8, KeySym::Numpad(NumpadKey::Num8)),
                (KeyCode::Kp9, KeySym::Numpad(NumpadKey::Num9)),
                (KeyCode::KpEnter, KeySym::Numpad(NumpadKey::Enter)),
                (KeyCode::KpPlus, KeySym::Numpad(NumpadKey::Plus)),
                (KeyCode::KpMinus, KeySym::Numpad(NumpadKey::Minus)),
                (KeyCode::KpAsterisk, KeySym::Numpad(NumpadKey::Asterisk)),
                (KeyCode::KpSlash, KeySym::Numpad(NumpadKey::Slash)),
            ],
        );

        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
            ],
            &[(KeyCode::KpDot, KeySym::Numpad(NumpadKey::Dot))],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::ALT],
            &[
                (KeyCode::Kp0, KeySym::AltNumpad(0x0)),
                (KeyCode::Kp1, KeySym::AltNumpad(0x1)),
                (KeyCode::Kp2, KeySym::AltNumpad(0x2)),
                (KeyCode::Kp3, KeySym::AltNumpad(0x3)),
                (KeyCode::Kp4, KeySym::AltNumpad(0x4)),
                (KeyCode::Kp5, KeySym::AltNumpad(0x5)),
                (KeyCode::Kp6, KeySym::AltNumpad(0x6)),
                (KeyCode::Kp7, KeySym::AltNumpad(0x7)),
                (KeyCode::Kp8, KeySym::AltNumpad(0x8)),
                (KeyCode::Kp9, KeySym::AltNumpad(0x9)),
            ],
        );
    }

    fn map_enter(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[(KeyCode::Enter, KeySym::Special(SpecialHandler::Enter))],
        );
    }

    fn map_compose(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::CTRL],
            &[(KeyCode::Dot, KeySym::Special(SpecialHandler::Compose))],
        );
    }

    fn map_modifier(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::LeftShift, KeySym::Modifier(ModifierKey::Shift)),
                (KeyCode::RightShift, KeySym::Modifier(ModifierKey::Shift)),
                (KeyCode::LeftCtrl, KeySym::Modifier(ModifierKey::Ctrl)),
                (KeyCode::RightCtrl, KeySym::Modifier(ModifierKey::Ctrl)),
                (KeyCode::LeftAlt, KeySym::Modifier(ModifierKey::Alt)),
                (KeyCode::RightAlt, KeySym::Modifier(ModifierKey::Alt)),
            ],
        );
    }

    fn map_reboot(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT],
            &[
                (KeyCode::Delete, KeySym::Special(SpecialHandler::Reboot)),
                (KeyCode::KpDot, KeySym::Special(SpecialHandler::Reboot)),
            ],
        );
    }

    fn map_scroll(&mut self) {
        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[
                (
                    KeyCode::PageUp,
                    KeySym::Special(SpecialHandler::ScrollBackward),
                ),
                (
                    KeyCode::PageDown,
                    KeySym::Special(SpecialHandler::ScrollForward),
                ),
            ],
        );
    }

    fn map_cursor_keys(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[
                (KeyCode::Up, KeySym::Cursor(CursorKey::Up)),
                (KeyCode::Down, KeySym::Cursor(CursorKey::Down)),
                (KeyCode::Left, KeySym::Cursor(CursorKey::Left)),
                (KeyCode::Right, KeySym::Cursor(CursorKey::Right)),
            ],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::ALT],
            &[
                (
                    KeyCode::Left,
                    KeySym::Special(SpecialHandler::DecreaseConsole),
                ),
                (
                    KeyCode::Right,
                    KeySym::Special(SpecialHandler::IncreaseConsole),
                ),
            ],
        );
    }

    fn map_lock_keys(&mut self) {
        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::SHIFT,
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[(
                KeyCode::CapsLock,
                KeySym::Special(SpecialHandler::ToggleCapsLock),
            )],
        );

        self.apply_key_binds(
            &[
                ModifierKeyFlags::empty(),
                ModifierKeyFlags::CTRL,
                ModifierKeyFlags::SHIFT | ModifierKeyFlags::CTRL,
                ModifierKeyFlags::ALT,
                ModifierKeyFlags::CTRL | ModifierKeyFlags::ALT,
            ],
            &[(
                KeyCode::NumLock,
                KeySym::Special(SpecialHandler::ToggleNumLock),
            )],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[(
                KeyCode::NumLock,
                KeySym::Special(SpecialHandler::ToggleBareNumLock),
            )],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::empty(), ModifierKeyFlags::ALT],
            &[(
                KeyCode::ScrollLock,
                KeySym::Special(SpecialHandler::ToggleScrollLock),
            )],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::SHIFT],
            &[(
                KeyCode::ScrollLock,
                KeySym::Special(SpecialHandler::ShowMem),
            )],
        );

        self.apply_key_binds(
            &[ModifierKeyFlags::CTRL],
            &[(
                KeyCode::ScrollLock,
                KeySym::Special(SpecialHandler::ShowState),
            )],
        );
    }
}

static KEYMAPS: Once<KeyMaps> = Once::new();

/// Look up the `KeySym` for a given modifier mask and key code.
pub(in crate::device::tty::vt::keyboard) fn get_keysym(
    mods: ModifierKeyFlags,
    key_code: KeyCode,
) -> KeySym {
    KEYMAPS
        .get()
        .expect("`KEYMAPS` is not initialized")
        .get_keysym(mods, key_code)
}

/// Function-string identifiers.
///
// Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/include/uapi/linux/keyboard.h#L49-L78>
#[expect(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::device::tty::vt::keyboard) enum FuncId {
    F1 = 0,
    F2 = 1,
    F3 = 2,
    F4 = 3,
    F5 = 4,
    F6 = 5,
    F7 = 6,
    F8 = 7,
    F9 = 8,
    F10 = 9,
    F11 = 10,
    F12 = 11,
    F13 = 12,
    F14 = 13,
    F15 = 14,
    F16 = 15,
    F17 = 16,
    F18 = 17,
    F19 = 18,
    F20 = 19,
    Find = 20,
    Insert = 21,
    Remove = 22,
    Select = 23,
    Prior = 24,
    Next = 25,
    Macro = 26,
    Help = 27,
    Do = 28,
    Pause = 29,
}

/// Default function-string table.
///
// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/defkeymap.c_shipped#L192-L224>
static FUNC_TABLE: [Option<&'static [u8]>; 30] = [
    Some(b"\x1b[[A"),
    Some(b"\x1b[[B"),
    Some(b"\x1b[[C"),
    Some(b"\x1b[[D"),
    Some(b"\x1b[[E"),
    Some(b"\x1b[17~"),
    Some(b"\x1b[18~"),
    Some(b"\x1b[19~"),
    Some(b"\x1b[20~"),
    Some(b"\x1b[21~"),
    Some(b"\x1b[23~"),
    Some(b"\x1b[24~"),
    Some(b"\x1b[25~"),
    Some(b"\x1b[26~"),
    Some(b"\x1b[28~"),
    Some(b"\x1b[29~"),
    Some(b"\x1b[31~"),
    Some(b"\x1b[32~"),
    Some(b"\x1b[33~"),
    Some(b"\x1b[34~"),
    Some(b"\x1b[1~"),
    Some(b"\x1b[2~"),
    Some(b"\x1b[3~"),
    Some(b"\x1b[4~"),
    Some(b"\x1b[5~"),
    Some(b"\x1b[6~"),
    Some(b"\x1b[M"),
    None,
    None,
    Some(b"\x1b[P"),
];

/// Get the function-string bytes for the given `FuncId`.
pub(in crate::device::tty::vt::keyboard) fn get_func_bytes(id: FuncId) -> Option<&'static [u8]> {
    FUNC_TABLE.get(id as usize).copied().flatten()
}

pub(super) fn init() {
    KEYMAPS.call_once(KeyMaps::default);
}
