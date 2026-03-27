// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::{AnyConsoleDevice, mode::ConsoleMode};
use ostd::sync::{LocalIrqDisabled, SpinLock, WaitQueue};
use spin::Once;

use crate::{
    device::{
        registry::char,
        tty::{
            Tty,
            vt::{
                MAX_CONSOLES, VtIndex, console::VtModeType, driver::VtDriver, ioctl_defs::CVtState,
            },
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug)]
struct VtManagerState {
    /// Current active VT index.
    active: VtIndex,
    /// Pending target VT during a switch request.
    wanted: Option<VtIndex>,
}

impl VtManagerState {
    fn new(active: VtIndex) -> Self {
        Self {
            active,
            wanted: None,
        }
    }

    /// The base VT index for relative switching.
    ///
    /// According to Linux behavior, if a switch is already pending (`wanted` is set),
    /// relative switches are computed from that pending target rather than
    /// the current active VT.
    fn base_for_relative_switch(&self) -> VtIndex {
        self.wanted.unwrap_or(self.active)
    }
}

pub(in crate::device::tty::vt) struct VtManager {
    terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES],
    state: SpinLock<VtManagerState, LocalIrqDisabled>,
    vt_active_wq: WaitQueue,
}

impl VtManager {
    const DEFAULT_ACTIVE_VT_INDEX: VtIndex = VtIndex::new(1).unwrap();

    fn new() -> Result<Self> {
        let mut terminals_vec = Vec::with_capacity(MAX_CONSOLES);

        for i in 0..MAX_CONSOLES {
            let driver = VtDriver::new();
            let vt_index = (i + 1) as u32;
            let tty = Tty::new(vt_index, driver);
            char::register(tty.clone())?;
            terminals_vec.push(tty);
        }

        let terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES] = terminals_vec
            .try_into()
            .unwrap_or_else(|_| panic!("`terminals_vec` length should be `MAX_CONSOLES`"));

        // Activate the first VT by default.
        terminals[Self::DEFAULT_ACTIVE_VT_INDEX.to_zero_based()]
            .driver()
            .try_switch_to_framebuffer_backend_and_activate();

        Ok(Self {
            terminals,
            state: SpinLock::new(VtManagerState::new(Self::DEFAULT_ACTIVE_VT_INDEX)),
            vt_active_wq: WaitQueue::new(),
        })
    }

    /// Gets an available VT index that is not allocated, or None if all VTs are allocated.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L823-L833>
    pub(in crate::device::tty::vt) fn get_available_vt_index(&self) -> Option<VtIndex> {
        let _state = self.state.lock();

        for i in 0..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if !self.is_in_use(idx) {
                return Some(idx);
            }
        }
        None
    }

    /// Gets the global VT state for ioctl responses.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L835-L854>
    pub(in crate::device::tty::vt) fn get_global_vt_state(&self) -> CVtState {
        let state = self.state.lock();

        let active_index = state.active;

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

    /// Switches to the target VT if it's different from the current one.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1215>
    pub(in crate::device::tty::vt) fn switch_vt(&self, target: VtIndex) -> Result<()> {
        let mut st = self.state.lock();

        if target == st.active {
            return Ok(());
        }

        let active_vt_console = self.vt(st.active).driver().vt_console();
        let active_console_mode = active_vt_console.mode();
        let active_mode_type = active_vt_console.inner().vt_mode().mode_type;

        // Validate target and active constraints.
        //
        // Mirrors Linux checks:
        // - target must be allocated,
        // - if active is in graphics mode and VT is in AUTO mode, switching is blocked.
        if !self.is_allocated(target)
            || (active_console_mode == Some(ConsoleMode::Graphics)
                && active_mode_type == VtModeType::Auto)
        {
            return_errno_with_message!(Errno::EINVAL, "invalid VT index");
        }

        st.wanted = Some(target);

        drop(st);

        // Process-controlled VT: notify user space and do not complete switch here.
        // Later, the process will respond with a `VT_RELDISP` ioctl.
        if active_mode_type == VtModeType::Process {
            active_vt_console.inner().set_wanted(Some(target));
            active_vt_console.send_release_signal();
            return Ok(());
        }

        self.complete_switch_vt(target)
    }

    /// Waits until the given VT becomes the active (foreground) VT.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L862-L870>
    pub(in crate::device::tty::vt) fn wait_for_vt_active(&self, target: VtIndex) -> Result<()> {
        self.vt_active_wq.wait_until(|| {
            let st = self.state.lock();
            if st.active == target { Some(()) } else { None }
        });
        Ok(())
    }

    /// Allocates the given VT.
    pub(in crate::device::tty::vt) fn allocate_vt(&self, index: VtIndex) -> Result<()> {
        self.vt(index)
            .driver()
            .vt_console()
            .try_switch_to_framebuffer_backend();
        Ok(())
    }

    /// Disallocates all VTs except VT 1 since it's the default VT and always open.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L648-L666>
    pub(in crate::device::tty::vt) fn disallocate_all_vts(&self) -> Result<()> {
        for i in 1..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if !self.is_busy(idx) {
                self.disallocate_vt(idx)?;
            }
        }
        Ok(())
    }

    /// Disallocates the specified VT if it's not busy.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L629-L646>
    pub(in crate::device::tty::vt) fn disallocate_vt(&self, index: VtIndex) -> Result<()> {
        if self.is_busy(index) {
            return_errno_with_message!(Errno::EBUSY, "VT is busy");
        } else if index.get() != 1 {
            self.vt(index)
                .driver()
                .vt_console()
                .switch_to_none_backend();
        }
        Ok(())
    }

    /// Switches to the previous allocated VT (wrap-around).
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L556>
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

    /// Switches to the next allocated VT (wrap-around).
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L573>
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

    /// Completes the VT switch to the target VT.
    ///
    /// References: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1141>
    pub(in crate::device::tty::vt) fn complete_switch_vt(&self, target: VtIndex) -> Result<()> {
        let (new_active_vt, need_acquire_signal) = {
            let mut st = self.state.lock();

            let active_console = self.vt(st.active).driver().vt_console();
            active_console.deactivate();
            st.active = target;

            let new_active_vt = self.vt(target);
            let new_mode_type = new_active_vt
                .driver()
                .vt_console()
                .inner()
                .vt_mode()
                .mode_type;
            (new_active_vt, new_mode_type == VtModeType::Process)
        };

        let new_active_console = new_active_vt.driver().vt_console();
        new_active_console.activate();
        log::info!("Switched to VT{}", new_active_vt.index());

        // Process-controlled VT: notify user space that it now owns the VT.
        if need_acquire_signal {
            new_active_console.send_acquire_signal();
        }

        self.state.lock().wanted = None;

        self.vt_active_wq.wake_all();
        Ok(())
    }

    /// Cancels the pending VT switch request.
    pub(in crate::device::tty::vt) fn cancel_switch_vt(&self) {
        self.state.lock().wanted = None;
    }

    #[inline]
    /// Gets the VT at the given index.
    fn vt(&self, index: VtIndex) -> Arc<Tty<VtDriver>> {
        self.terminals[index.to_zero_based()].clone()
    }

    /// Checks if the VT at `index` is allocated.
    fn is_allocated(&self, index: VtIndex) -> bool {
        !self.vt(index).driver().vt_console().is_backend_none()
    }

    /// Checks if the VT at `index` is currently in use.
    fn is_in_use(&self, index: VtIndex) -> bool {
        self.vt(index).driver().is_open()
    }

    /// Checks if the VT at `index` is busy, meaning it's either in use or the active VT.
    fn is_busy(&self, index: VtIndex) -> bool {
        self.is_in_use(index) || index == self.state.lock().active
    }

    /// Gets the currently active VT.
    fn active_vt(&self) -> Arc<Tty<VtDriver>> {
        let state = self.state.lock();
        self.vt(state.active)
    }
}

pub(in crate::device::tty::vt) static VT_MANAGER: Once<VtManager> = Once::new();

pub(super) fn init_in_first_process() -> Result<()> {
    let vtm = VtManager::new()?;
    VT_MANAGER.call_once(|| vtm);
    Ok(())
}

pub fn active_vt() -> Arc<Tty<VtDriver>> {
    let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
    vtm.active_vt()
}
