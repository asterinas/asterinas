// SPDX-License-Identifier: MPL-2.0

//! CPU-related definitions.

pub mod local;
pub mod set;

pub use set::{AtomicCpuSet, CpuSet};

pub use crate::arch::cpu::*;
use crate::{cpu_local_cell, task::atomic_mode::InAtomicMode};

/// The ID of a CPU in the system.
///
/// If converting from/to an integer, the integer must start from 0 and be less
/// than the number of CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuId(u32);

impl CpuId {
    /// Returns the CPU ID of the bootstrap processor (BSP).
    pub const fn bsp() -> Self {
        CpuId(0)
    }

    /// Converts the CPU ID to an `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns the ID of the current CPU.
    ///
    /// This function is safe to call, but is vulnerable to races. The returned CPU
    /// ID may be outdated if the task migrates to another CPU.
    ///
    /// To ensure that the CPU ID is up-to-date, do it under any guards that
    /// implement the [`PinCurrentCpu`] trait.
    pub fn current_racy() -> Self {
        #[cfg(debug_assertions)]
        assert!(IS_CURRENT_CPU_INITED.load());

        Self(CURRENT_CPU.load())
    }
}

/// The error type returned when converting an out-of-range integer to [`CpuId`].
#[derive(Debug, Clone, Copy)]
pub struct CpuIdFromIntError;

impl TryFrom<usize> for CpuId {
    type Error = CpuIdFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value < num_cpus() {
            Ok(CpuId(value as u32))
        } else {
            Err(CpuIdFromIntError)
        }
    }
}

/// The number of CPUs.
static mut NUM_CPUS: u32 = 1;

/// Initializes the number of CPUs.
///
/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The argument is the correct value of the number of CPUs (which
///    is a constant, since we don't support CPU hot-plugging anyway).
unsafe fn init_num_cpus(num_cpus: u32) {
    assert!(num_cpus >= 1);

    // SAFETY: It is safe to mutate this global variable because we
    // are in the boot context.
    unsafe { NUM_CPUS = num_cpus };

    // Note that decreasing the number of CPUs may break existing
    // `CpuId`s (which have a type invariant to say that the ID is
    // less than the number of CPUs).
    //
    // However, this never happens: due to the safety conditions
    // it's only legal to call this function to increase the number
    // of CPUs from one (the initial value) to the actual number of
    // CPUs.
}

/// Returns the number of CPUs.
pub fn num_cpus() -> usize {
    // SAFETY: As far as the safe APIs are concerned, `NUM_CPUS` is
    // read-only, so it is always valid to read.
    (unsafe { NUM_CPUS }) as usize
}

/// Returns an iterator over all CPUs.
pub fn all_cpus() -> impl Iterator<Item = CpuId> {
    (0..num_cpus()).map(|id| CpuId(id as u32))
}

cpu_local_cell! {
    /// The current CPU ID.
    static CURRENT_CPU: u32 = 0;
    /// The initialization state of the current CPU ID.
    #[cfg(debug_assertions)]
    static IS_CURRENT_CPU_INITED: bool = false;
}

/// Initializes the current CPU ID.
///
/// # Safety
///
/// This method must be called on each processor during the early
/// boot phase of the processor.
///
/// The caller must ensure that this function is called with
/// the correct value of the CPU ID.
unsafe fn set_this_cpu_id(id: u32) {
    // FIXME: If there are safe APIs that rely on the correctness of
    // the CPU ID for soundness, we'd better make the CPU ID a global
    // invariant and initialize it before entering `ap_early_entry`.
    CURRENT_CPU.store(id);

    #[cfg(debug_assertions)]
    IS_CURRENT_CPU_INITED.store(true);
}

/// A marker trait for guard types that can "pin" the current task to the
/// current CPU.
///
/// Such guard types include [`DisabledLocalIrqGuard`] and
/// [`DisabledPreemptGuard`]. When such guards exist, the CPU executing the
/// current task is pinned. So getting the current CPU ID or CPU-local
/// variables are safe.
///
/// # Safety
///
/// The implementor must ensure that the current task is pinned to the current
/// CPU while any one of the instances of the implemented structure exists.
///
/// [`DisabledLocalIrqGuard`]: crate::trap::irq::DisabledLocalIrqGuard
/// [`DisabledPreemptGuard`]: crate::task::DisabledPreemptGuard
pub unsafe trait PinCurrentCpu {
    /// Returns the ID of the current CPU.
    fn current_cpu(&self) -> CpuId {
        CpuId::current_racy()
    }
}

// SAFETY: A guard that enforces the atomic mode requires disabling any
// context switching. So naturally, the current task is pinned on the CPU.
unsafe impl<T: InAtomicMode> PinCurrentCpu for T {}
unsafe impl PinCurrentCpu for dyn InAtomicMode + '_ {}

/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The number of available processors is available.
/// 3. No CPU-local objects have been accessed.
pub(crate) unsafe fn init_on_bsp() {
    let num_cpus = crate::arch::boot::smp::count_processors().unwrap_or(1);

    // SAFETY: The safety is upheld by the caller and
    // the correctness of the `get_num_processors` method.
    unsafe {
        local::copy_bsp_for_ap(num_cpus as usize);

        set_this_cpu_id(0);

        // Note that `init_num_cpus` should be called after `copy_bsp_for_ap`.
        // This helps to build the safety reasoning in `CpuLocal::get_on_cpu`.
        // See its implementation for details.
        init_num_cpus(num_cpus);
    }
}

/// # Safety
///
/// The caller must ensure that:
/// 1. We're in the boot context of an AP.
/// 2. The CPU ID of the AP is `cpu_id`.
pub(crate) unsafe fn init_on_ap(cpu_id: u32) {
    // SAFETY: The safety is upheld by the caller.
    unsafe { set_this_cpu_id(cpu_id) };
}
