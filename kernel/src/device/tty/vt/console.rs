// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use aster_console::{
    AnyConsoleDevice, ConsoleCallback, ConsoleSetFontError,
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
};
use aster_framebuffer::{FRAMEBUFFER, FramebufferConsole};
use int_to_c_enum::TryFromInt;
use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};

use crate::{
    device::tty::vt::{VtIndex, keyboard::VtKeyboard},
    process::{Process, signal::sig_num::SigNum},
};

/// The VT mode type.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub(super) enum VtModeType {
    /// Kernel-controlled switching.
    ///
    /// In this mode, the VT automatically switches without notifying any process.
    Auto = 0,
    /// Process-controlled switching.
    ///
    /// In this mode, a process is notified when the VT is switched away from or to it.
    /// The process can then release or acquire the VT as needed.
    Process = 1,
}

/// The VT mode configuration.
///
/// The meaning mirrors Linux `struct vt_mode`.
#[derive(Debug, Clone, Copy)]
pub(super) struct VtMode {
    pub mode_type: VtModeType,
    pub wait_on_inactive: bool,
    pub release_signal: Option<SigNum>,
    pub acquire_signal: Option<SigNum>,
}

impl Default for VtMode {
    fn default() -> Self {
        Self {
            mode_type: VtModeType::Auto,
            wait_on_inactive: false,
            release_signal: None,
            acquire_signal: None,
        }
    }
}

/// The VT console backend.
pub(super) enum VtConsoleBackend {
    /// Framebuffer exists and is used for this VT console.
    Framebuffer(FramebufferConsole),
    /// No framebuffer device available or Framebuffer exists but this VT console
    /// is not initialized.
    None,
}

/// The inner of [`VtConsole`].
pub(super) struct VtConsoleInner {
    mode: VtMode,
    process: Option<Weak<Process>>,
    wanted: Option<VtIndex>,
}

impl VtConsoleInner {
    fn new() -> Self {
        Self {
            mode: VtMode::default(),
            process: None,
            wanted: None,
        }
    }

    /// Gets the VT mode configuration.
    pub(super) fn vt_mode(&self) -> &VtMode {
        &self.mode
    }

    /// Sets the VT mode configuration.
    pub(super) fn set_vt_mode(&mut self, vt_mode: VtMode) {
        self.mode = vt_mode;
    }

    /// Sets the controlling process ID.
    pub(super) fn set_process(&mut self, process: Option<Weak<Process>>) {
        self.process = process;
    }

    /// Gets the wanted switch VT index.
    pub(super) fn wanted(&self) -> Option<VtIndex> {
        self.wanted
    }

    /// Sets the wanted switch VT index.
    pub(super) fn set_wanted(&mut self, wanted: Option<VtIndex>) {
        self.wanted = wanted;
    }
}

/// The virtual terminal console.
pub(super) struct VtConsole {
    keyboard: SpinLock<VtKeyboard, LocalIrqDisabled>,
    backend: SpinLock<VtConsoleBackend, LocalIrqDisabled>,
    inner: SpinLock<VtConsoleInner, LocalIrqDisabled>,
}

impl core::fmt::Debug for VtConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VtConsole").finish_non_exhaustive()
    }
}

impl VtConsole {
    /// Creates a new VT console with none backend.
    pub(super) fn new() -> Self {
        Self {
            keyboard: SpinLock::new(VtKeyboard::default()),
            backend: SpinLock::new(VtConsoleBackend::None),
            inner: SpinLock::new(VtConsoleInner::new()),
        }
    }

    /// Locks the inner and gets a guard.
    pub(super) fn lock_inner(&self) -> SpinLockGuard<'_, VtConsoleInner, LocalIrqDisabled> {
        self.inner.lock()
    }

    /// Locks the keyboard and gets a guard.
    pub(super) fn lock_keyboard(&self) -> SpinLockGuard<'_, VtKeyboard, LocalIrqDisabled> {
        self.keyboard.lock()
    }

    /// Marks this VT as active.
    pub(super) fn activate(&self) {
        let mut backend = self.backend.lock();
        if let VtConsoleBackend::Framebuffer(c) = &mut *backend {
            c.activate();
        }
    }

    /// Marks this VT as inactive.
    pub(super) fn deactivate(&self) {
        let mut backend = self.backend.lock();
        if let VtConsoleBackend::Framebuffer(c) = &mut *backend {
            c.deactivate();
        }
    }

    /// Delivers the configured `release` signal to the controlling process.
    ///
    /// This is used when switching away from this VT in [`VtModeType::Process`] mode.
    pub(super) fn send_release_signal(&self) {
        let inner = self.inner.lock();
        if let (Some(signum), Some(process)) = (inner.mode.release_signal, inner.process.clone()) {
            crate::process::enqueue_signal_async(process, signum);
        }
    }

    /// Delivers the configured `acquire` signal to the controlling process.
    ///
    /// This is used when switching to this VT in [`VtModeType::Process`] mode.
    pub(super) fn send_acquire_signal(&self) {
        let inner = self.inner.lock();
        if let (Some(signum), Some(process)) = (inner.mode.acquire_signal, inner.process.clone()) {
            crate::process::enqueue_signal_async(process, signum);
        }
    }

    /// Tries to switch to framebuffer backend if the framebuffer is available.
    pub(super) fn try_switch_to_framebuffer_backend(&self) {
        if matches!(*(self.backend.lock()), VtConsoleBackend::Framebuffer(_)) {
            return;
        }
        if let Some(fb) = FRAMEBUFFER.get() {
            *self.backend.lock() =
                VtConsoleBackend::Framebuffer(FramebufferConsole::new(fb.clone()));
        }
    }

    /// Switches to none backend.
    pub(super) fn switch_to_none_backend(&self) {
        *self.backend.lock() = VtConsoleBackend::None;
    }
}

impl AnyConsoleDevice for VtConsole {
    fn send(&self, buf: &[u8]) {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => c.send(buf),
            VtConsoleBackend::None => {}
        }
    }

    fn register_callback(&self, _callback: &'static ConsoleCallback) {}

    fn set_font(&self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => c.set_font(font),
            VtConsoleBackend::None => Err(ConsoleSetFontError::InappropriateDevice),
        }
    }

    fn set_mode(&self, mode: ConsoleMode) -> bool {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => {
                c.set_mode(mode);
                true
            }
            VtConsoleBackend::None => false,
        }
    }

    fn mode(&self) -> Option<ConsoleMode> {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => Some(c.mode()),
            VtConsoleBackend::None => None,
        }
    }

    fn set_keyboard_mode(&self, mode: KeyboardMode) -> bool {
        let mut keyboard = self.keyboard.lock();
        keyboard.set_mode(mode)
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        Some(self.keyboard.lock().mode())
    }
}
