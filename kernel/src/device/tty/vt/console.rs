// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use aster_console::{
    AnyConsoleDevice, ConsoleCallback, ConsoleSetFontError,
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
};
use aster_framebuffer::{
    ConsoleState, DummyFramebufferConsole, EscapeFsm, FRAMEBUFFER, FrameBuffer,
};
use int_to_c_enum::TryFromInt;
use ostd::sync::{LocalIrqDisabled, SpinLock};

use crate::{
    device::tty::vt::keyboard::VtKeyboard,
    process::{
        Pid,
        signal::{sig_num::SigNum, signals::kernel::KernelSignal},
    },
};

/// The VT mode type.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub(in crate::device::tty::vt) enum VtModeType {
    /// Kernel-controlled switching.
    ///
    /// In this mode, the VT automatically switches without notifying any process.
    Auto = 0,
    /// Process-controlled switching.
    ///
    /// In this mode, a process is notified when the VT is switched away from or to it. The process
    /// can then release or acquire the VT as needed.
    Process = 1,
}

/// The VT mode configuration.
///
/// The meaning mirrors Linux `struct vt_mode`.
#[derive(Debug, Clone, Copy)]
pub(in crate::device::tty::vt) struct VtMode {
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

/// The virtual terminal console device.
pub(in crate::device::tty::vt) struct VtConsole {
    keyboard: VtKeyboard,
    mode: SpinLock<VtMode>,
    pid: SpinLock<Option<Pid>>,
    inner: SpinLock<(ConsoleState, EscapeFsm), LocalIrqDisabled>,
}

impl core::fmt::Debug for VtConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VtConsole").finish_non_exhaustive()
    }
}

impl VtConsole {
    /// Create a new VT console backed by a real framebuffer.
    fn new(framebuffer: Arc<FrameBuffer>) -> Self {
        let state = ConsoleState::new(framebuffer);
        let esc_fsm = EscapeFsm::new();
        Self {
            keyboard: VtKeyboard::default(),
            mode: SpinLock::new(VtMode::default()),
            pid: SpinLock::new(None),
            inner: SpinLock::new((state, esc_fsm)),
        }
    }

    /// Mark this VT as active.
    ///
    /// For [VtModeType::Auto] mode, we enable rendering and redraw the full shadow buffer
    /// so the framebuffer immediately shows the latest text contents for this VT.
    pub(in crate::device::tty::vt) fn activate(&self) {
        if self.vt_mode().mode_type == VtModeType::Auto {
            let mut inner = self.inner.lock();
            inner.0.enable_rendering();
            inner.0.flush_fullscreen();
        }
    }

    /// Mark this VT as inactive.
    ///
    /// For [VtModeType::Auto] mode, we disable rendering so background VTs keep
    /// their shadow buffers updated but do not draw to the physical framebuffer.
    pub(in crate::device::tty::vt) fn deactivate(&self) {
        if self.vt_mode().mode_type == VtModeType::Auto {
            let mut inner = self.inner.lock();
            inner.0.disable_rendering();
        }
    }

    /// Get a reference to the VT keyboard device.
    pub(in crate::device::tty::vt) fn vt_keyboard(&self) -> &VtKeyboard {
        &self.keyboard
    }

    /// Get the VT mode configuration.
    pub(in crate::device::tty::vt) fn vt_mode(&self) -> VtMode {
        let guard = self.mode.lock();
        *guard
    }

    /// Set the VT mode configuration.
    pub(in crate::device::tty::vt) fn set_vt_mode(&self, mode: VtMode) {
        let mut guard = self.mode.lock();
        *guard = mode;
    }

    /// Get the controlling process ID.
    fn pid(&self) -> Option<Pid> {
        let guard = self.pid.lock();
        *guard
    }

    /// Set the controlling process ID.
    pub(in crate::device::tty::vt) fn set_pid(&self, pid: Option<Pid>) {
        let mut guard = self.pid.lock();
        *guard = pid;
    }

    /// Deliver the configured "release" signal to the controlling process, if any.
    ///
    /// This is used when switching away from this VT in [VtModeType::Process] mode.
    pub(in crate::device::tty::vt) fn send_release_signal(&self) {
        let mode = self.vt_mode();
        if let (Some(sig), Some(pid)) = (mode.release_signal, self.pid())
            && let Some(process) = crate::process::process_table::get_process(pid)
        {
            process.enqueue_signal(Box::new(KernelSignal::new(sig)));
        }
    }

    /// Deliver the configured "acquire" signal to the controlling process, if any.
    ///
    /// This is used when switching to this VT in [VtModeType::Process] mode.
    pub(in crate::device::tty::vt) fn send_acquire_signal(&self) {
        let mode = self.vt_mode();
        if let (Some(sig), Some(pid)) = (mode.acquire_signal, self.pid())
            && let Some(process) = crate::process::process_table::get_process(pid)
        {
            process.enqueue_signal(Box::new(KernelSignal::new(sig)));
        }
    }
}

impl AnyConsoleDevice for VtConsole {
    fn send(&self, buf: &[u8]) {
        let mut inner = self.inner.lock();
        let (state, esc_fsm) = &mut *inner;

        for byte in buf {
            if esc_fsm.eat(*byte, state) {
                // The character is part of an ANSI escape sequence.
                continue;
            }

            if *byte == 0 {
                // The character is a NUL character.
                continue;
            }

            state.send_char(*byte);
        }
    }

    fn register_callback(&self, _callback: &'static ConsoleCallback) {}

    fn set_font(&self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        self.inner.lock().0.set_font(font)
    }

    fn set_mode(&self, mode: ConsoleMode) -> bool {
        self.inner.lock().0.set_mode(mode);
        true
    }

    fn mode(&self) -> Option<ConsoleMode> {
        Some(self.inner.lock().0.mode())
    }

    fn set_keyboard_mode(&self, mode: KeyboardMode) -> bool {
        self.keyboard.set_mode(mode);
        true
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        Some(self.keyboard.mode())
    }
}

#[derive(Debug)]
pub(in crate::device::tty::vt) enum VtConsoleBackend {
    /// Framebuffer exists and is usable.
    Real(Arc<VtConsole>),
    /// No framebuffer device available; or not yet initialized.
    Dummy(Arc<DummyFramebufferConsole>),
}

impl VtConsoleBackend {
    fn console(&self) -> &dyn AnyConsoleDevice {
        match self {
            VtConsoleBackend::Real(c) => c.as_ref(),
            VtConsoleBackend::Dummy(c) => c.as_ref(),
        }
    }
}

/// The proxy console device for virtual terminal.
#[derive(Debug)]
pub(in crate::device::tty::vt) struct VtConsoleProxy {
    backend: SpinLock<VtConsoleBackend, LocalIrqDisabled>,
}

impl VtConsoleProxy {
    pub(in crate::device::tty::vt) fn new_dummy() -> Self {
        Self {
            backend: SpinLock::new(VtConsoleBackend::Dummy(Arc::new(DummyFramebufferConsole))),
        }
    }

    /// Try to upgrade the backend to a real VT console if the framebuffer is available.
    pub(in crate::device::tty::vt) fn try_upgrade_to_real(&self) {
        let mut guard = self.backend.lock();
        if matches!(&*guard, VtConsoleBackend::Real(_)) {
            return;
        }
        if let Some(fb) = FRAMEBUFFER.get() {
            *guard = VtConsoleBackend::Real(Arc::new(VtConsole::new(fb.clone())));
        }
    }

    /// Try to downgrade the backend to dummy if it's currently real.
    pub(in crate::device::tty::vt) fn try_downgrade_to_dummy(&self) {
        let mut guard = self.backend.lock();
        if matches!(&*guard, VtConsoleBackend::Dummy(_)) {
            return;
        }
        *guard = VtConsoleBackend::Dummy(Arc::new(DummyFramebufferConsole));
    }

    /// Get a reference to the real VT console if currently in Real backend.
    pub(in crate::device::tty::vt) fn vt_console(&self) -> Option<Arc<VtConsole>> {
        let guard = self.backend.lock();
        match &*guard {
            VtConsoleBackend::Real(c) => Some(c.clone()),
            VtConsoleBackend::Dummy(_) => None,
        }
    }
}

impl AnyConsoleDevice for VtConsoleProxy {
    fn send(&self, buf: &[u8]) {
        self.backend.lock().console().send(buf);
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.backend.lock().console().register_callback(callback);
    }

    fn set_font(&self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        self.backend.lock().console().set_font(font)
    }

    fn set_mode(&self, mode: ConsoleMode) -> bool {
        self.backend.lock().console().set_mode(mode)
    }

    fn mode(&self) -> Option<ConsoleMode> {
        self.backend.lock().console().mode()
    }

    fn set_keyboard_mode(&self, mode: KeyboardMode) -> bool {
        self.backend.lock().console().set_keyboard_mode(mode)
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        self.backend.lock().console().keyboard_mode()
    }
}
