// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::{AnyConsoleDevice, mode::ConsoleMode};
use ostd::sync::{SpinLock, WaitQueue};
use spin::Once;

use crate::{
    device::{
        registry::char,
        tty::{
            Tty,
            vt::{
                MAX_CONSOLES, VtIndex,
                console::VtModeType,
                driver::VtDriver,
                ioctl_defs::{CVtState, ReleaseDisplayType},
            },
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug)]
struct VtSwitchState {
    /// Current foreground VT index.
    active: VtIndex,
    /// Pending target VT during a switch request.
    wanted: Option<VtIndex>,
}

impl VtSwitchState {
    fn new(active: VtIndex) -> Self {
        Self {
            active,
            wanted: None,
        }
    }

    /// Base VT for relative switching.
    ///
    /// According to Linux behavior, if a switch is already pending (`wanted` is set),
    /// relative switches are computed from that pending target rather than
    /// the current foreground VT.
    fn base_for_relative_switch(&self) -> VtIndex {
        self.wanted.unwrap_or(self.active)
    }
}

pub(in crate::device::tty::vt) struct VirtualTerminalManager {
    terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES],
    state: SpinLock<VtSwitchState>,
    vt_active_wq: WaitQueue,
}

impl VirtualTerminalManager {
    fn new() -> crate::prelude::Result<Self> {
        let mut terminals: [Option<Arc<Tty<VtDriver>>>; MAX_CONSOLES] =
            core::array::from_fn(|_| None);

        for (i, item) in terminals.iter_mut().enumerate().take(MAX_CONSOLES) {
            let driver = VtDriver::new();
            let tty = Tty::new((i + 1) as u32, driver);
            char::register(tty.clone())?;
            *item = Some(tty);
        }

        let terminals = terminals.map(|opt| opt.unwrap());

        // Activate the first VT by default.
        terminals[0].driver().try_upgrade_to_real_and_activate();

        Ok(Self {
            terminals,
            state: SpinLock::new(VtSwitchState::new(VtIndex::new(1).unwrap())),
            vt_active_wq: WaitQueue::new(),
        })
    }

    #[inline]
    fn vt(&self, index: VtIndex) -> Arc<Tty<VtDriver>> {
        self.terminals[index.to_zero_based()].clone()
    }

    /// Checks if the VT at `index` is allocated.
    fn is_allocated(&self, index: VtIndex) -> bool {
        self.vt(index).driver().vt_console().is_some()
    }

    /// Checks if the VT at `index` is currently in use.
    fn is_in_use(&self, index: VtIndex) -> bool {
        self.is_allocated(index) && self.vt(index).driver().is_open()
    }

    fn active_vt(&self) -> Arc<Tty<VtDriver>> {
        let index = self.state.lock().active;
        self.vt(index)
    }

    pub(in crate::device::tty::vt) fn get_available_vt_index(&self) -> Option<VtIndex> {
        for i in 0..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if !self.is_in_use(idx) {
                return Some(idx);
            }
        }
        None
    }

    pub(in crate::device::tty::vt) fn get_global_vt_state(&self) -> CVtState {
        let active_index = self.state.lock().active;

        // `/dev/tty0` is always open.
        let mut state_bits: u16 = 1;
        // The `state_bits` is u16, so it can represent at most 16 VTs and the `/dev/tty0` bit usage.
        let max_track = (u16::BITS as usize).saturating_sub(1).min(MAX_CONSOLES);

        for i in 0..max_track {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if self.is_in_use(idx) {
                state_bits |= 1 << (i + 1);
            }
        }

        CVtState {
            active: active_index.get() as u16,
            signal: 0,
            state: state_bits,
        }
    }

    pub(in crate::device::tty::vt) fn switch_vt(&self, target: VtIndex) -> Result<()> {
        let mut st = self.state.lock();

        if target == st.active {
            st.wanted = None;
            return Ok(());
        }

        let active_vt = self.vt(st.active);
        let active_vt_console =
            active_vt
                .driver()
                .vt_console()
                .ok_or(crate::prelude::Error::with_message(
                    Errno::EINVAL,
                    "VT console not found",
                ))?;

        // Validate target and active constraints.
        //
        // Mirrors Linux checks:
        // - target must be allocated,
        // - if active is in graphics mode and VT is in AUTO mode, switching is blocked.
        if !self.is_allocated(target)
            || (active_vt_console.mode() == Some(ConsoleMode::Graphics)
                && active_vt_console.vt_mode().mode_type == VtModeType::Auto)
        {
            return_errno_with_message!(Errno::EINVAL, "invalid VT index");
        }

        st.wanted = Some(target);

        drop(st);

        // Process-controlled VT: notify user space and do not complete switch here.
        if active_vt_console.vt_mode().mode_type == VtModeType::Process {
            active_vt_console.send_release_signal();
            return Ok(());
        }

        self.complete_switch_vt()
    }

    pub(in crate::device::tty::vt) fn complete_switch_vt(&self) -> Result<()> {
        let (new_active_vt, need_acquire_signal) = {
            let mut st = self.state.lock();
            let target = match st.wanted.take() {
                Some(t) => t,
                None => return Ok(()),
            };

            self.vt(st.active)
                .driver()
                .vt_console()
                .ok_or(crate::prelude::Error::with_message(
                    Errno::EINVAL,
                    "VT console not found",
                ))?
                .deactivate();
            st.active = target;

            let wanted_vt = self.vt(target);
            let wanted_mode_type = wanted_vt
                .driver()
                .vt_console()
                .ok_or(crate::prelude::Error::with_message(
                    Errno::EINVAL,
                    "VT console not found",
                ))?
                .vt_mode()
                .mode_type;
            (wanted_vt, wanted_mode_type == VtModeType::Process)
        };

        let new_active_vt_console =
            new_active_vt
                .driver()
                .vt_console()
                .ok_or(crate::prelude::Error::with_message(
                    Errno::EINVAL,
                    "VT console not found",
                ))?;
        new_active_vt_console.activate();
        log::info!("Switched to VT{}", new_active_vt.index());

        // Process-controlled VT: notify user space that it now owns the VT.
        if need_acquire_signal {
            new_active_vt_console.send_acquire_signal();
        }

        self.vt_active_wq.wake_all();
        Ok(())
    }

    /// Switch to the previous allocated VT (wrap-around).
    pub(in crate::device::tty::vt) fn dec_console(&self) -> Result<()> {
        let cur = self.state.lock().base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.prev_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt(i);
            }
        }
    }

    /// Switch to the next allocated VT (wrap-around).
    pub(in crate::device::tty::vt) fn inc_console(&self) -> Result<()> {
        let cur = self.state.lock().base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.next_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt(i);
            }
        }
    }

    /// Handles the release display response from the current VT when it's in process-controlled mode.
    pub(in crate::device::tty::vt) fn handle_reldisp(
        &self,
        reldisp_type: ReleaseDisplayType,
    ) -> Result<()> {
        let (pending, in_process_mode) = {
            let st = self.state.lock();
            let active_vt = self.vt(st.active);
            let vc = active_vt
                .driver()
                .vt_console()
                .ok_or(crate::prelude::Error::with_message(
                    Errno::EINVAL,
                    "VT console not found",
                ))?;

            (st.wanted, vc.vt_mode().mode_type == VtModeType::Process)
        };

        if !in_process_mode {
            return_errno_with_message!(Errno::EINVAL, "VT is not in process mode");
        }

        if pending.is_none() {
            // If it's just an ACK, ignore it.
            return if reldisp_type == ReleaseDisplayType::AckAcquire {
                Ok(())
            } else {
                return_errno_with_message!(Errno::EINVAL, "no VT switch is pending");
            };
        }

        if reldisp_type == ReleaseDisplayType::DenyRelease {
            // Switch disallowed, so forget we were trying to do it.
            let mut st = self.state.lock();
            st.wanted = None;
            return Ok(());
        }

        // The current VT has been released, so complete the switch.
        self.complete_switch_vt()?;
        Ok(())
    }

    /// Wait until the given VT becomes the active (foreground) VT.
    pub(in crate::device::tty::vt) fn wait_for_vt_active(&self, target: VtIndex) -> Result<()> {
        self.vt_active_wq.wait_until(|| {
            let st = self.state.lock();
            if st.active == target { Some(()) } else { None }
        });
        Ok(())
    }
}

pub(in crate::device::tty::vt) static VIRTUAL_TERMINAL_MANAGER: Once<VirtualTerminalManager> =
    Once::new();

pub(super) fn init() -> Result<()> {
    let vtm = VirtualTerminalManager::new()?;
    VIRTUAL_TERMINAL_MANAGER.call_once(|| vtm);
    Ok(())
}

pub fn active_vt() -> Arc<Tty<VtDriver>> {
    let vtm = VIRTUAL_TERMINAL_MANAGER
        .get()
        .expect("`VIRTUAL_TERMINAL_MANAGER` is not initialized");
    vtm.active_vt()
}
