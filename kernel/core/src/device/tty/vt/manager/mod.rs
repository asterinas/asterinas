// SPDX-License-Identifier: MPL-2.0

mod console;

use aster_framebuffer::mode::ConsoleMode;
use console::VtModeType;
pub(super) use console::{VtConsole, VtMode};
use ostd::sync::{LocalIrqDisabled, WaitQueue};
use spin::Once;

use crate::{
    device::{
        registry::char,
        tty::{
            Tty,
            vt::{
                MAX_CONSOLES, VtIndex,
                c_types::{CVtState, ReleaseDisplayType},
                driver::VtDriver,
            },
        },
    },
    prelude::*,
};

pub(super) struct VtManager {
    terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES],
    /// Protects `VtManagerState` data and serializes VT manager operations.
    state: SpinLock<VtManagerState, LocalIrqDisabled>,
    vt_active_wq: WaitQueue,
}

/// Lock-holding facade for VT manager operations.
pub(super) struct VtManagerGuard<'a> {
    manager: &'a VtManager,
    state: SpinLockGuard<'a, VtManagerState, LocalIrqDisabled>,
}

#[derive(Clone, Copy, Debug)]
struct VtManagerState {
    /// Current active VT index.
    active: VtIndex,
    /// Pending VT switch target requested from the active VT in [`VtModeType::Process`] mode.
    wanted: Option<VtIndex>,
    /// Per-VT activation generation counter. Incremented whenever a VT becomes active.
    gens: [u64; MAX_CONSOLES],
}

impl VtManagerState {
    fn new(active: VtIndex) -> Self {
        let mut gens = [0u64; MAX_CONSOLES];
        gens[active.to_zero_based()] = 1;

        Self {
            active,
            wanted: None,
            gens,
        }
    }

    /// Returns the base VT index for relative switching.
    ///
    /// According to Linux behavior, if a switch is already pending (`wanted` is set),
    /// relative switches are computed from that pending target rather than
    /// the current foreground VT.
    fn base_for_relative_switch(&self) -> VtIndex {
        self.wanted.unwrap_or(self.active)
    }
}

impl VtManager {
    /// The minimum default allocated VT index.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/vt.h#L12>
    const MIN_DEFAULT_ALLOCATED_VT_INDEX: VtIndex = VtIndex::new(1).unwrap();

    /// The default active VT index.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3750>
    const DEFAULT_ACTIVE_VT_INDEX: VtIndex = VtIndex::new(1).unwrap();

    fn new() -> Result<Self> {
        let terminals = core::array::try_from_fn(|i| {
            let vt_index = VtIndex::new((i + 1) as u8).unwrap();
            let driver = VtDriver::new(vt_index);
            let tty = Tty::new(vt_index.get() as u32, driver);
            char::register(tty.clone())?;
            Ok::<Arc<Tty<VtDriver>>, Error>(tty)
        })?;

        // Initialize default allocated VTs.
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3741-L3749>
        for i in 0..Self::MIN_DEFAULT_ALLOCATED_VT_INDEX.get() {
            terminals[i as usize].driver().vt_console().allocate();
        }

        // Activate the default VT.
        terminals[Self::DEFAULT_ACTIVE_VT_INDEX.to_zero_based()]
            .driver()
            .vt_console()
            .activate();

        Ok(Self {
            terminals,
            state: SpinLock::new(VtManagerState::new(Self::DEFAULT_ACTIVE_VT_INDEX)),
            vt_active_wq: WaitQueue::new(),
        })
    }

    /// Acquires the VT manager lock and returns a lock-holding facade.
    pub(super) fn lock(&self) -> VtManagerGuard<'_> {
        VtManagerGuard {
            manager: self,
            state: self.state.lock(),
        }
    }

    /// Waits until the given VT becomes the active VT.
    ///
    /// Note:
    /// - This is used for the `VT_WAITACTIVE` ioctl command.
    pub(super) fn wait_for_vt_active(&self, target: VtIndex) -> Result<()> {
        let state_lock = self.state.lock();

        // Fast path: already active.
        if state_lock.active == target {
            return Ok(());
        }

        // Capture the current generation for the target VT. We will return when the
        // generation changes, which indicates the target became active at least once
        // since we started waiting. This avoids a race where the VT becomes active
        // and then quickly switches away before this thread wakes up.
        let start_gen = state_lock.gens[target.to_zero_based()];

        drop(state_lock);

        self.vt_active_wq.pause_until(|| {
            let state_lock = self.state.lock();
            if state_lock.gens[target.to_zero_based()] != start_gen {
                Some(())
            } else {
                None
            }
        })?;

        Ok(())
    }

    /// Gets the currently active VT.
    pub(super) fn active_vt(&self) -> &Arc<Tty<VtDriver>> {
        let state_lock = self.state.lock();
        self.vt(state_lock.active)
    }

    /// Gets the VT at the given index.
    fn vt(&self, index: VtIndex) -> &Arc<Tty<VtDriver>> {
        &self.terminals[index.to_zero_based()]
    }
}

impl VtManagerGuard<'_> {
    // Lock order: manager -> console keyboard/backend/inner

    /// Gets an available VT index that is not allocated, or `None` if all VTs are allocated.
    ///
    /// Note:
    /// - This is used for the `VT_OPENQRY` ioctl command.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L823-L833>
    pub(super) fn get_available_vt_index(&self) -> Option<VtIndex> {
        for i in 0..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if !self.is_in_use(idx) {
                return Some(idx);
            }
        }
        None
    }

    /// Sets the VT mode and controlling process for the given VT.
    ///
    /// Note:
    /// - This is used for the `VT_SETMODE` ioctl command.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L762-L783>
    pub(super) fn set_vt_mode(&mut self, index: VtIndex, vt_mode: VtMode) {
        // Linux behavior: `VT_SETMODE` clears the pending VT switch target.
        // We follow the same userspace-visible behavior.
        if index == self.state.active {
            self.state.wanted = None;
        }

        self.manager
            .vt(index)
            .driver()
            .vt_console()
            .set_vt_mode_and_process(vt_mode, Some(Arc::downgrade(&current!())));
    }

    /// Sets the console mode for the given VT.
    pub(super) fn set_console_mode(&self, index: VtIndex, mode: ConsoleMode) {
        self.manager.vt(index).driver().vt_console().set_mode(mode);
    }

    /// Gets the global VT state for ioctl responses.
    ///
    /// Note:
    /// - This is used for the `VT_GETSTATE` ioctl command.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L800-L821>
    pub(super) fn get_global_vt_state(&self) -> CVtState {
        let active_index = self.state.active;

        // `/dev/tty0` is always open.
        let mut state_bits: u16 = 1;
        // The `state_bits` is a `u16`, so it can represent `/dev/tty0` and at most 15 VTs.
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
    /// Note:
    /// - This is used for the `VT_ACTIVATE` ioctl command and keyboard-driven VT switching.
    pub(super) fn switch_vt(&mut self, target: VtIndex) -> Result<()> {
        self.switch_vt_inner(target)
    }

    /// Handles the `ReleaseDisplay` ioctl command.
    ///
    /// Note:
    /// - This is used for the `VT_RELDISP` ioctl command.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L555-L589>
    pub(super) fn handle_release_display(
        &mut self,
        vt_index: VtIndex,
        reldisp_type: ReleaseDisplayType,
    ) -> Result<()> {
        let vt_console = self.manager.vt(vt_index).driver().vt_console();
        let mode_type = vt_console.lock_inner().vt_mode().mode_type();
        if mode_type != VtModeType::Process {
            return_errno_with_message!(Errno::EINVAL, "the VT is not in process mode");
        }

        if vt_index != self.state.active {
            if reldisp_type == ReleaseDisplayType::AckAcquire {
                return Ok(());
            } else {
                return_errno_with_message!(Errno::EINVAL, "the VT is not active");
            }
        }

        let Some(wanted) = self.state.wanted else {
            if reldisp_type == ReleaseDisplayType::AckAcquire {
                return Ok(());
            } else {
                return_errno_with_message!(Errno::EINVAL, "no VT switch is pending");
            };
        };

        if reldisp_type == ReleaseDisplayType::DenyRelease {
            self.state.wanted = None;
            return Ok(());
        }

        self.state.wanted = None;

        self.allocate_vt(wanted);

        self.complete_switch_vt(wanted)
    }

    /// Allocates the given VT.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L1053>
    pub(super) fn allocate_vt(&self, index: VtIndex) {
        self.manager.vt(index).driver().vt_console().allocate();
    }

    /// Disallocates all VTs except VT1 since it's the default VT and always open.
    ///
    /// Note:
    /// - This is used for the `VT_DISALLOCATE` ioctl command with `index` set to 0.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L648-L666>
    pub(super) fn disallocate_all_vts(&self) {
        for i in 1..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            let _ = self.disallocate_vt(idx);
        }
    }

    /// Disallocates the specified VT if it's not busy.
    ///
    /// Note:
    /// - This is used for the `VT_DISALLOCATE` ioctl command with `index`
    ///   set to a specific VT index.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L629-L646>
    pub(super) fn disallocate_vt(&self, index: VtIndex) -> Result<()> {
        if self.is_busy(index) {
            return_errno_with_message!(Errno::EBUSY, "the VT is busy");
        }

        if index.get() != 1
            && self.is_allocated(index)
            && index > VtManager::MIN_DEFAULT_ALLOCATED_VT_INDEX
        {
            self.manager.vt(index).driver().vt_console().disallocate();
        }

        Ok(())
    }

    /// Switches to the previous allocated VT (wrap-around).
    ///
    /// Note:
    /// - This is used for the `SpecialHandler::DecreaseConsole` keyboard handler.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L556>
    pub(super) fn dec_console(&mut self) -> Result<()> {
        let cur = self.state.base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.prev_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i);
            }
        }
    }

    /// Switches to the next allocated VT (wrap-around).
    ///
    /// Note:
    /// - This is used for the `SpecialHandler::IncreaseConsole` keyboard handler.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L573>
    pub(super) fn inc_console(&mut self) -> Result<()> {
        let cur = self.state.base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.next_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i);
            }
        }
    }

    /// Checks if the VT at `index` is allocated.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L993-L996>
    fn is_allocated(&self, index: VtIndex) -> bool {
        self.manager.vt(index).driver().vt_console().is_allocated()
    }

    /// Checks if the VT at `index` is currently in use.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L44-L55>
    fn is_in_use(&self, index: VtIndex) -> bool {
        let driver = self.manager.vt(index).driver();
        driver.vt_console().is_allocated() && driver.open_file_counter().has_open_files(self)
    }

    /// Checks if the VT at `index` is busy, meaning it's either in use or the active VT.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L57-L67>
    fn is_busy(&self, index: VtIndex) -> bool {
        self.is_in_use(index) || index == self.state.active
    }

    /// Switches to the target VT if it's different from the current one.
    ///
    /// References:
    /// - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3235-L3257>
    /// - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3191-L3233>
    /// - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1212-L1280>
    fn switch_vt_inner(&mut self, target: VtIndex) -> Result<()> {
        if target == self.state.active {
            return Ok(());
        }

        let active_index = self.state.active;
        let (active_console_mode, active_mode_type) = {
            let active_vt_console = self.manager.vt(active_index).driver().vt_console();
            (
                active_vt_console.mode(),
                active_vt_console.lock_inner().vt_mode().mode_type(),
            )
        };

        // Validate target and active constraints.
        //
        // This mirrors Linux checks:
        // - `target` must be allocated,
        // - if `active` is in graphics mode and VT switching is controlled by the kernel,
        //   switching is blocked.
        if !self.is_allocated(target)
            || (active_console_mode == ConsoleMode::Graphics
                && active_mode_type == VtModeType::Auto)
        {
            return_errno_with_message!(Errno::EINVAL, "VT switching is rejected");
        }

        // Process-controlled VT: notify user space and do not complete switch here.
        // Later, the process will respond with a `VT_RELDISP` ioctl.
        if active_mode_type == VtModeType::Process {
            self.state.wanted = Some(target);
            self.manager
                .vt(active_index)
                .driver()
                .vt_console()
                .send_release_signal();
            return Ok(());
        }

        self.complete_switch_vt(target)
    }

    /// Completes the VT switch to the target VT.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1137-L1210>
    fn complete_switch_vt(&mut self, target: VtIndex) -> Result<()> {
        let old_active_console = self.manager.vt(self.state.active).driver().vt_console();
        old_active_console.deactivate();

        self.state.active = target;
        self.state.gens[target.to_zero_based()] =
            self.state.gens[target.to_zero_based()].wrapping_add(1);

        let new_active_console = self.manager.vt(target).driver().vt_console();
        new_active_console.activate();

        ostd::info!("Switched to VT{}", target.get());

        // Process-controlled VT: notify user space that it now owns the VT.
        if new_active_console.lock_inner().vt_mode().mode_type() == VtModeType::Process {
            new_active_console.send_acquire_signal();
        }

        self.manager.vt_active_wq.wake_all();

        Ok(())
    }
}

pub(super) static VT_MANAGER: Once<VtManager> = Once::new();

pub(super) fn init_in_first_process() -> Result<()> {
    let vtm = VtManager::new()?;
    VT_MANAGER.call_once(|| vtm);
    Ok(())
}

pub fn active_vt() -> &'static Arc<Tty<VtDriver>> {
    let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
    vtm.active_vt()
}

pub(super) fn default_vt() -> &'static Arc<Tty<VtDriver>> {
    let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
    vtm.vt(VtManager::DEFAULT_ACTIVE_VT_INDEX)
}
