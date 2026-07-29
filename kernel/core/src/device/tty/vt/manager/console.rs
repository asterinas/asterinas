// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::sync::atomic::{AtomicBool, Ordering};

use aster_framebuffer::{
    console::FramebufferConsole, font::BitmapFont, framebuffer::FRAMEBUFFER, mode::ConsoleMode,
};
use int_to_c_enum::TryFromInt;
use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};

use crate::{
    device::tty::vt::{c_types::CVtMode, keyboard::VtKeyboard},
    error::{Errno, Error, return_errno_with_message},
    process::{Process, signal::sig_num::SigNum},
};

/// The virtual terminal console.
pub(in crate::device::tty::vt) struct VtConsole {
    keyboard: SpinLock<VtKeyboard, LocalIrqDisabled>,
    is_allocated: AtomicBool,
    backend: SpinLock<VtConsoleBackend, LocalIrqDisabled>,
    inner: SpinLock<VtConsoleInner, LocalIrqDisabled>,
}

/// The VT console backend.
enum VtConsoleBackend {
    /// Framebuffer exists and is used for this VT console.
    Framebuffer(FramebufferConsole),
    /// No framebuffer device is available or framebuffer exists but this VT console
    /// is not initialized.
    None,
}

/// The inner of [`VtConsole`].
pub(in crate::device::tty::vt) struct VtConsoleInner {
    mode: VtMode,
    process: Option<Weak<Process>>,
    console_mode: ConsoleMode,
}

impl VtConsoleInner {
    fn new() -> Self {
        Self {
            mode: VtMode::default(),
            process: None,
            console_mode: ConsoleMode::Text,
        }
    }

    /// Gets the VT mode configuration.
    pub(in crate::device::tty::vt) fn vt_mode(&self) -> &VtMode {
        &self.mode
    }
}

/// The VT mode configuration.
#[derive(Clone, Copy, Debug)]
pub(in crate::device::tty::vt) struct VtMode {
    mode_type: VtModeType,
    wait_on_inactive: bool,
    release_signal: Option<SigNum>,
    acquire_signal: Option<SigNum>,
}

impl VtMode {
    /// Gets the VT mode type.
    pub(super) fn mode_type(&self) -> VtModeType {
        self.mode_type
    }
}

/// The VT mode type.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(in crate::device::tty::vt) enum VtModeType {
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

impl From<VtMode> for CVtMode {
    fn from(mode: VtMode) -> Self {
        CVtMode {
            mode: mode.mode_type as u8,
            waitv: mode.wait_on_inactive as u8,
            relsig: mode.release_signal.map_or(0, |s| s.as_u8() as u16),
            acqsig: mode.acquire_signal.map_or(0, |s| s.as_u8() as u16),
            frsig: 0,
        }
    }
}

impl TryInto<VtMode> for CVtMode {
    type Error = Error;

    fn try_into(self) -> crate::prelude::Result<VtMode> {
        let mode_type = VtModeType::try_from(self.mode)?;
        let wait_on_inactive = self.waitv != 0;

        // Linux treats out-of-range signal numbers in `struct vt_mode` by
        // ignoring them rather than returning `EINVAL`. We follow that behavior:
        // - 0 means "no signal";
        // - invalid/unknown signal numbers are treated as 0 (`None`) instead of
        //   returning an error.
        let release_signal = if self.relsig == 0 {
            None
        } else {
            SigNum::try_from(self.relsig as u8).ok()
        };
        let acquire_signal = if self.acqsig == 0 {
            None
        } else {
            SigNum::try_from(self.acqsig as u8).ok()
        };

        Ok(VtMode {
            mode_type,
            wait_on_inactive,
            release_signal,
            acquire_signal,
        })
    }
}

impl core::fmt::Debug for VtConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VtConsole").finish_non_exhaustive()
    }
}

impl VtConsole {
    /// Creates a new VT console with none backend.
    pub(in crate::device::tty::vt) fn new() -> Self {
        Self {
            keyboard: SpinLock::new(VtKeyboard::default()),
            is_allocated: AtomicBool::new(false),
            backend: SpinLock::new(VtConsoleBackend::None),
            inner: SpinLock::new(VtConsoleInner::new()),
        }
    }

    /// Locks the inner and gets a guard.
    pub(in crate::device::tty::vt) fn lock_inner(
        &self,
    ) -> SpinLockGuard<'_, VtConsoleInner, LocalIrqDisabled> {
        self.inner.lock()
    }

    /// Locks the keyboard and gets a guard.
    pub(in crate::device::tty::vt) fn lock_keyboard(
        &self,
    ) -> SpinLockGuard<'_, VtKeyboard, LocalIrqDisabled> {
        self.keyboard.lock()
    }

    /// Gets the console mode.
    pub(in crate::device::tty::vt) fn mode(&self) -> ConsoleMode {
        let inner = self.inner.lock();
        inner.console_mode
    }

    /// Sends the given buffer to this console.
    pub(in crate::device::tty::vt) fn send(&self, buf: &[u8]) {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => c.send(buf),
            VtConsoleBackend::None => {}
        }
    }

    /// Sets the console font.
    pub(in crate::device::tty::vt) fn set_font(
        &self,
        font: BitmapFont,
    ) -> crate::prelude::Result<()> {
        let mut backend = self.backend.lock();
        match &mut *backend {
            VtConsoleBackend::Framebuffer(c) => c.set_font(font).map_err(|_| {
                Error::with_message(Errno::EINVAL, "the font is invalid for the console")
            }),
            VtConsoleBackend::None => {
                return_errno_with_message!(
                    Errno::ENOTTY,
                    "the console has no support for font setting"
                );
            }
        }
    }
}

impl VtConsole {
    // These methods are called under the VT manager lock, so it is race-free to
    // take multiple internal locks and drop them mid-method.

    /// Sets the VT mode configuration.
    pub(super) fn set_vt_mode_and_process(&self, vt_mode: VtMode, process: Option<Weak<Process>>) {
        let mut inner = self.inner.lock();
        inner.mode = vt_mode;
        inner.process = process;
    }

    /// Sets the console mode.
    pub(super) fn set_mode(&self, mode: ConsoleMode) {
        {
            let mut inner = self.inner.lock();
            inner.console_mode = mode;
        }

        let mut backend = self.backend.lock();
        if let VtConsoleBackend::Framebuffer(c) = &mut *backend {
            c.set_mode(mode);
        }
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
    ///
    /// FIXME: Report whether delivery is possible so the manager can reset
    /// this VT to [`VtModeType::Auto`] instead of leaving switch handoff pending forever
    /// if the process is dead or cannot receive signals.
    pub(super) fn send_release_signal(&self) {
        let inner = self.inner.lock();
        if let (Some(signum), Some(process)) = (inner.mode.release_signal, inner.process.clone()) {
            drop(inner);
            crate::process::enqueue_signal_async(process, signum);
        }
    }

    /// Delivers the configured `acquire` signal to the controlling process.
    ///
    /// This is used when switching to this VT in [`VtModeType::Process`] mode.
    ///
    /// FIXME: Report whether delivery is possible so the manager can reset
    /// this VT to [`VtModeType::Auto`] and clear stale process-controlled ownership state
    /// if the target process is gone or cannot receive signals.
    pub(super) fn send_acquire_signal(&self) {
        let inner = self.inner.lock();
        if let (Some(signum), Some(process)) = (inner.mode.acquire_signal, inner.process.clone()) {
            drop(inner);
            crate::process::enqueue_signal_async(process, signum);
        }
    }

    /// Allocates this VT.
    pub(super) fn allocate(&self) {
        let mode = {
            let inner = self.inner.lock();
            inner.console_mode
        };

        let mut backend = self.backend.lock();
        if !matches!(*backend, VtConsoleBackend::Framebuffer(_))
            && let Some(fb) = FRAMEBUFFER.get()
        {
            let mut console = FramebufferConsole::new(fb.clone());
            console.set_mode(mode);
            *backend = VtConsoleBackend::Framebuffer(console);
        }
        self.is_allocated.store(true, Ordering::Relaxed);
    }

    /// Disallocates this VT.
    pub(super) fn disallocate(&self) {
        // Linux frees the object that keeps per-VT state on `VT_DISALLOCATE`;
        // we reset per-VT state here to match fresh-allocation behavior.
        *self.keyboard.lock() = VtKeyboard::default();
        *self.inner.lock() = VtConsoleInner::new();

        let mut backend = self.backend.lock();
        *backend = VtConsoleBackend::None;
        self.is_allocated.store(false, Ordering::Relaxed);
    }

    /// Checks whether this VT is allocated.
    pub(super) fn is_allocated(&self) -> bool {
        self.is_allocated.load(Ordering::Relaxed)
    }
}
