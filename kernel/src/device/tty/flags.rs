// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

/// TTY flags.
//
// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/linux/tty.h#L306>.
pub struct TtyFlags {
    is_other_closed: AtomicBool,
    is_pty_locked: AtomicBool,
}

impl TtyFlags {
    pub fn new() -> Self {
        Self {
            is_other_closed: AtomicBool::new(false),
            is_pty_locked: AtomicBool::new(false),
        }
    }

    pub fn set_other_closed(&self) {
        self.is_other_closed.store(true, Ordering::Relaxed);
    }

    pub fn clear_other_closed(&self) {
        self.is_other_closed.store(false, Ordering::Relaxed);
    }

    pub fn is_other_closed(&self) -> bool {
        self.is_other_closed.load(Ordering::Relaxed)
    }

    pub fn set_pty_locked(&self) {
        self.is_pty_locked.store(true, Ordering::Relaxed);
    }

    pub fn clear_pty_locked(&self) {
        self.is_pty_locked.store(false, Ordering::Relaxed);
    }

    pub fn is_pty_locked(&self) -> bool {
        self.is_pty_locked.load(Ordering::Relaxed)
    }
}
