// SPDX-License-Identifier: MPL-2.0

mod handler;
mod keysym;

use core::sync::atomic::{AtomicU8, Ordering};

use aster_framebuffer::mode::{KeyboardMode, KeyboardModeFlags};

use crate::prelude::*;

/// Initializes the key symbol mapping table and keyboard event handler.
pub(super) fn init_in_first_process() {
    keysym::init_in_first_process();
    handler::init_in_first_process();
}

/// A modifier key (Shift, Ctrl, Alt).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModifierKey {
    Shift,
    Ctrl,
    Alt,
}

/// A numpad key (0-9, Dot, Enter, Plus, Minus, Asterisk, Slash).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumpadKey {
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Dot,
    Enter,
    Plus,
    Minus,
    Asterisk,
    Slash,
}

/// A cursor key (Up, Down, Left, Right).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CursorKey {
    Up,
    Down,
    Left,
    Right,
}

bitflags::bitflags! {
    /// A set of currently active modifier keys.
    struct ModifierKeyFlags: u8 {
        const SHIFT     = 1 << 0;
        const ALT       = 1 << 1;
        const CTRL      = 1 << 2;
    }

    /// A set of currently enabled lock keys.
    struct LockKeyFlags: u8 {
        const CAPS_LOCK   = 1 << 0;
        const NUM_LOCK    = 1 << 1;
        const SCROLL_LOCK = 1 << 2;
    }
}

/// State of modifier keys (Shift, Ctrl, Alt).
#[derive(Debug)]
struct ModifierKeysState {
    inner: AtomicU8,
}

impl ModifierKeysState {
    const fn new() -> Self {
        Self {
            inner: AtomicU8::new(0),
        }
    }

    /// Marks the given modifier keys as pressed.
    fn press(&self, keys: ModifierKeyFlags) {
        self.inner.fetch_or(keys.bits(), Ordering::Relaxed);
    }

    /// Marks the given modifier keys as released.
    fn release(&self, keys: ModifierKeyFlags) {
        self.inner.fetch_and(!keys.bits(), Ordering::Relaxed);
    }

    /// Returns the currently active modifier keys.
    fn flags(&self) -> ModifierKeyFlags {
        ModifierKeyFlags::from_bits_truncate(self.inner.load(Ordering::Relaxed))
    }
}

/// The keyboard state for each virtual terminal.
pub(super) struct VtKeyboard {
    lock_key_flags: LockKeyFlags,
    mode: KeyboardMode,
    mode_flags: KeyboardModeFlags,
}

impl Default for VtKeyboard {
    fn default() -> Self {
        Self {
            lock_key_flags: LockKeyFlags::empty(),
            mode: KeyboardMode::Unicode,
            // Linux default: REPEAT | META
            // Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/drivers/tty/vt/keyboard.c#L56>
            mode_flags: KeyboardModeFlags::REPEAT | KeyboardModeFlags::META,
        }
    }
}

impl VtKeyboard {
    /// Returns the current lock key flags.
    fn lock_key_flags(&self) -> LockKeyFlags {
        self.lock_key_flags
    }

    /// Toggles the given lock keys.
    fn toggle_lock_keys(&mut self, keys: LockKeyFlags) {
        self.lock_key_flags.toggle(keys);
    }

    /// Returns the current keyboard mode.
    pub(super) fn mode(&self) -> KeyboardMode {
        self.mode
    }

    /// Sets the keyboard mode.
    pub(super) fn set_mode(&mut self, mode: KeyboardMode) -> Result<()> {
        match mode {
            // TODO: Add support for Raw mode.
            KeyboardMode::Raw => {
                return_errno_with_message!(Errno::EINVAL, "the keyboard mode is not supported")
            }
            KeyboardMode::Xlate
            | KeyboardMode::MediumRaw
            | KeyboardMode::Unicode
            | KeyboardMode::Off => {
                self.mode = mode;
                Ok(())
            }
        }
    }

    /// Returns the current keyboard mode flags.
    pub(super) fn mode_flags(&self) -> KeyboardModeFlags {
        self.mode_flags
    }
}
