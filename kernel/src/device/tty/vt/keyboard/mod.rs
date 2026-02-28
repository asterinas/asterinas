// SPDX-License-Identifier: MPL-2.0

mod handler;
mod keysym;

use core::sync::atomic::{AtomicU8, Ordering};

use aster_console::mode::{KeyboardMode, KeyboardModeFlags};
use ostd::sync::SpinLock;

/// Initialize the key symbol mapping table and keyboard event handler.
pub(super) fn init() {
    keysym::init();
    handler::init();
}

/// A modifier key (Shift, Ctrl, Alt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifierKey {
    Shift,
    Ctrl,
    Alt,
}

/// A numpad key (0-9, Dot, Enter, Plus, Minus, Asterisk, Slash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

bitflags::bitflags! {
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

/// State of lock keys (Caps Lock, Num Lock, Scroll Lock).
#[derive(Debug)]
pub(super) struct LockKeysState {
    inner: AtomicU8,
}

impl LockKeysState {
    const fn new() -> Self {
        Self {
            inner: AtomicU8::new(0),
        }
    }

    /// Toggles the given lock keys.
    fn toggle(&self, keys: LockKeyFlags) {
        self.inner.fetch_xor(keys.bits(), Ordering::Relaxed);
    }

    /// Returns the currently enabled lock keys.
    fn flags(&self) -> LockKeyFlags {
        LockKeyFlags::from_bits_truncate(self.inner.load(Ordering::Relaxed))
    }
}

/// The keyboard state for each virtual terminal.
pub(super) struct VtKeyboard {
    lock_keys_state: LockKeysState,
    mode: SpinLock<KeyboardMode>,
    mode_flags: SpinLock<KeyboardModeFlags>,
}

impl Default for VtKeyboard {
    fn default() -> Self {
        Self {
            lock_keys_state: LockKeysState::new(),
            mode: SpinLock::new(KeyboardMode::Unicode),
            // Linux default: REPEAT | META
            // Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/drivers/tty/vt/keyboard.c#L56>
            mode_flags: SpinLock::new(KeyboardModeFlags::REPEAT | KeyboardModeFlags::META),
        }
    }
}

impl VtKeyboard {
    /// Returns the state of lock keys.
    pub(super) fn lock_keys_state(&self) -> &LockKeysState {
        &self.lock_keys_state
    }

    /// Returns the current keyboard mode.
    pub(super) fn mode(&self) -> KeyboardMode {
        *self.mode.lock()
    }

    /// Sets the keyboard mode.
    pub(super) fn set_mode(&self, mode: KeyboardMode) {
        let mut guard = self.mode.lock();
        *guard = mode;
    }

    /// Returns the current keyboard mode flags.
    pub(super) fn mode_flags(&self) -> KeyboardModeFlags {
        let guard = self.mode_flags.lock();
        *guard
    }

    /// Sets the keyboard mode flags.
    #[expect(dead_code)]
    pub(super) fn set_mode_flags(&self, flags: KeyboardModeFlags) {
        let mut guard = self.mode_flags.lock();
        *guard = flags;
    }
}
