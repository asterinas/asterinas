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
                MAX_CONSOLES, VtIndex,
                c_types::{CVtState, ReleaseDisplayType},
                console::VtModeType,
                driver::VtDriver,
            },
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug)]
struct VtManagerState {
    /// Current active VT index.
    active: VtIndex,
    /// Per-VT activation generation counter. Incremented whenever a VT becomes active.
    gens: [u64; MAX_CONSOLES],
}

impl VtManagerState {
    fn new(active: VtIndex) -> Self {
        let mut gens = [0u64; MAX_CONSOLES];
        gens[active.to_zero_based()] = 1;

        Self { active, gens }
    }
}

pub(super) struct VtManager {
    terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES],
    state: SpinLock<VtManagerState, LocalIrqDisabled>,
    lock: SpinLock<(), LocalIrqDisabled>,
    vt_active_wq: WaitQueue,
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
            terminals[i as usize].driver().initialize();
        }

        // Activate the default VT.
        terminals[Self::DEFAULT_ACTIVE_VT_INDEX.to_zero_based()]
            .driver()
            .vt_console()
            .activate();

        Ok(Self {
            terminals,
            state: SpinLock::new(VtManagerState::new(Self::DEFAULT_ACTIVE_VT_INDEX)),
            lock: SpinLock::new(()),
            vt_active_wq: WaitQueue::new(),
        })
    }

    /// Gets an available VT index that is not allocated, or `None` if all VTs are allocated.
    ///
    /// Note:
    ///     - This is used for the `VT_OPENQRY` ioctl command.
    ///     - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L823-L833>
    pub(super) fn get_available_vt_index(&self) -> Option<VtIndex> {
        debug_assert!(self.lock.try_lock().is_none());

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
    /// Note:
    ///     - This is used for the `VT_GETSTATE` ioctl command.
    ///     - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L800-L821>
    pub(super) fn get_global_vt_state(&self) -> CVtState {
        debug_assert!(self.lock.try_lock().is_none());

        let active_index = self.state.lock().active;

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
    ///     - This is used for the `VT_ACTIVATE` ioctl command and keyboard-driven VT switching.
    ///     - This should be called in [`VtManager::with_lock`].
    pub(super) fn switch_vt(&self, target: VtIndex) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        let mut state_lock = self.state.lock();

        self.switch_vt_inner(target, &mut state_lock)
    }

    /// Handles the `ReleaseDisplay` ioctl command.
    ///
    /// Note:
    ///    - This is used for the `VT_RELDISP` ioctl command.
    ///    - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L555-L589>
    pub(super) fn handle_release_display(
        &self,
        vt_index: VtIndex,
        reldisp_type: ReleaseDisplayType,
    ) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        let mut state_lock = self.state.lock();

        let vt_console = self.vt(vt_index).driver().vt_console();
        let mut vt_console_inner = vt_console.lock_inner();
        let mode_type = vt_console_inner.vt_mode().mode_type;
        if mode_type != VtModeType::Process {
            return_errno_with_message!(Errno::EINVAL, "the VT is not in process mode");
        }

        let Some(wanted) = vt_console_inner.wanted() else {
            if reldisp_type == ReleaseDisplayType::AckAcquire {
                return Ok(());
            } else {
                return_errno_with_message!(Errno::EINVAL, "no VT switch is pending");
            };
        };

        if reldisp_type == ReleaseDisplayType::DenyRelease {
            vt_console_inner.set_wanted(None);
            return Ok(());
        }

        vt_console_inner.set_wanted(None);

        self.allocate_vt(wanted)?;

        self.complete_switch_vt(wanted, &mut state_lock)
    }

    /// Waits until the given VT becomes the active VT.
    ///
    /// Note:
    ///    - This is used for the `VT_WAITACTIVE` ioctl command.
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

    /// Allocates the given VT.
    ///
    /// Note:
    ///     - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L1053>
    pub(super) fn allocate_vt(&self, index: VtIndex) -> Result<()> {
        // FIXME: The Linux kernel does more things here, but we don't have the exact
        // same abstractions, so we just switch to the framebuffer backend.
        debug_assert!(self.lock.try_lock().is_none());

        let vt_driver = self.vt(index).driver();
        if !vt_driver.is_initialized() {
            vt_driver.initialize();
        }

        Ok(())
    }

    /// Disallocates all VTs except VT 1 since it's the default VT and always open.
    ///
    /// Note:
    ///     - This is used for the `VT_DISALLOCATE` ioctl command with `index` set to 0.
    ///     - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L648-L666>
    pub(super) fn disallocate_all_vts(&self) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        for i in 1..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            self.disallocate_vt(idx)?
        }

        Ok(())
    }

    /// Disallocates the specified VT if it's not busy.
    ///
    /// Note:
    ///     - This is used for the `VT_DISALLOCATE` ioctl command with `index`
    ///       set to a specific VT index.
    ///     - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L629-L646>
    pub(super) fn disallocate_vt(&self, index: VtIndex) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        let state_lock = self.state.lock();

        if self.is_busy(index, &state_lock) {
            return_errno_with_message!(Errno::EBUSY, "the VT is busy");
        } else if index.get() != 1 {
            let driver = self.vt(index).driver();
            driver.vt_console().switch_to_none_backend();
            if index > Self::MIN_DEFAULT_ALLOCATED_VT_INDEX {
                driver.dec_ref_count();
            }
        }

        Ok(())
    }

    /// Switches to the previous allocated VT (wrap-around).
    ///
    /// Note:
    ///    - This is used for the `SpecialHandler::DecreaseConsole` keyboard handler.
    ///    - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L556>
    pub(super) fn dec_console(&self) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        let mut state_lock = self.state.lock();
        let cur = state_lock.active;

        let mut i = cur;
        loop {
            i = i.prev_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i, &mut state_lock);
            }
        }
    }

    /// Switches to the next allocated VT (wrap-around).
    ///
    /// Note:
    ///    - This is used for the `SpecialHandler::IncreaseConsole` keyboard handler.
    ///    - This should be called in [`VtManager::with_lock`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/keyboard.c#L573>
    pub(super) fn inc_console(&self) -> Result<()> {
        debug_assert!(self.lock.try_lock().is_none());

        let mut state_lock = self.state.lock();
        let cur = state_lock.active;

        let mut i = cur;
        loop {
            i = i.next_wrap();

            if i == cur {
                // No other VT is available or allocated.
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i, &mut state_lock);
            }
        }
    }

    /// Acquires the lock for VT manager operations and runs the given closure with
    /// the lock held.
    ///
    /// This is used to serialize operations that need to read or modify the
    /// VT manager state, such as switching VTs or allocating/disallocating VTs.
    pub(super) fn with_lock<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VtManager) -> R,
    {
        let _guard = self.lock.lock();
        f(self)
    }

    /// Switches to the target VT if it's different from the current one.
    ///
    /// References:
    ///     - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3235-L3257>
    ///     - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3191-L3233>
    ///     - <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1212-L1280>
    fn switch_vt_inner(
        &self,
        target: VtIndex,
        state_lock: &mut SpinLockGuard<'_, VtManagerState, LocalIrqDisabled>,
    ) -> Result<()> {
        if target == state_lock.active {
            return Ok(());
        }

        let active_vt_console = self.vt(state_lock.active).driver().vt_console();
        let active_console_mode = active_vt_console.mode();
        let active_mode_type = active_vt_console.lock_inner().vt_mode().mode_type;

        // Validate target and active constraints.
        //
        // This mirrors Linux checks:
        // - `target` must be allocated,
        // - if `active` is in graphics mode and VT switching is controlled by the kernel,
        //   switching is blocked.
        if !self.is_allocated(target)
            || (active_console_mode == Some(ConsoleMode::Graphics)
                && active_mode_type == VtModeType::Auto)
        {
            return_errno_with_message!(Errno::EINVAL, "VT switching is rejected");
        }

        // Process-controlled VT: notify user space and do not complete switch here.
        // Later, the process will respond with a `VT_RELDISP` ioctl.
        if active_mode_type == VtModeType::Process {
            active_vt_console.lock_inner().set_wanted(Some(target));
            active_vt_console.send_release_signal();
            return Ok(());
        }

        self.complete_switch_vt(target, state_lock)
    }

    /// Completes the VT switch to the target VT.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L1137-L1210>
    fn complete_switch_vt(
        &self,
        target: VtIndex,
        state_lock: &mut SpinLockGuard<'_, VtManagerState, LocalIrqDisabled>,
    ) -> Result<()> {
        let old_active_console = self.vt(state_lock.active).driver().vt_console();
        old_active_console.deactivate();

        state_lock.active = target;
        state_lock.gens[target.to_zero_based()] =
            state_lock.gens[target.to_zero_based()].saturating_add(1);

        let new_active_console = self.vt(target).driver().vt_console();
        new_active_console.activate();

        log::info!("Switched to VT{}", target.get());

        // Process-controlled VT: notify user space that it now owns the VT.
        if new_active_console.lock_inner().vt_mode().mode_type == VtModeType::Process {
            new_active_console.send_acquire_signal();
        }

        self.vt_active_wq.wake_all();

        Ok(())
    }

    /// Gets the VT at the given index.
    fn vt(&self, index: VtIndex) -> &Arc<Tty<VtDriver>> {
        &self.terminals[index.to_zero_based()]
    }

    /// Checks if the VT at `index` is allocated.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L993-L996>
    fn is_allocated(&self, index: VtIndex) -> bool {
        self.vt(index).driver().is_initialized()
    }

    /// Checks if the VT at `index` is currently in use.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L44-L55>
    fn is_in_use(&self, index: VtIndex) -> bool {
        // We should hold this lock to prevent the VT from being disallocated while
        // we're checking if it's in use.
        debug_assert!(self.lock.try_lock().is_none());

        self.vt(index).driver().is_in_use()
    }

    /// Checks if the VT at `index` is busy, meaning it's either in use or the active VT.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L57-L67>
    fn is_busy(
        &self,
        index: VtIndex,
        state_lock: &SpinLockGuard<'_, VtManagerState, LocalIrqDisabled>,
    ) -> bool {
        self.is_in_use(index) || index == state_lock.active
    }

    /// Gets the currently active VT.
    fn active_vt(&self) -> &Arc<Tty<VtDriver>> {
        let state = self.state.lock();
        self.vt(state.active)
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
