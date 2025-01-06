// SPDX-License-Identifier: MPL-2.0

//! CPU-related definitions.

pub mod local;
pub mod set;

pub use set::{AtomicCpuSet, CpuSet};

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        pub use crate::arch::x86::cpu::*;
    } else if #[cfg(target_arch = "riscv64")] {
        pub use crate::arch::riscv::cpu::*;
    }
}

use crate::{cpu_local_cell, task::DisabledPreemptGuard, trap::DisabledLocalIrqGuard};

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
}

impl TryFrom<usize> for CpuId {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value < num_cpus() {
            Ok(CpuId(value as u32))
        } else {
            Err("The given CPU ID is out of range")
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
    /// The ID of the current CPU.
    static CURRENT_CPU: u32 = 0;
}

/// Initializes the ID of the current CPU.
///
/// This method only needs to be called on application processors.
///
/// # Safety
///
/// The caller must ensure that this function is called with
/// the correct value of the CPU ID.
unsafe fn set_this_cpu_id(id: u32) {
    CURRENT_CPU.store(id);
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
pub unsafe trait PinCurrentCpu {
    /// Returns the number of the current CPU.
    fn current_cpu(&self) -> CpuId {
        CpuId(CURRENT_CPU.load())
    }
}

// SAFETY: When IRQs are disabled, the task cannot be passively preempted and
// migrates to another CPU. If the task actively calls `yield`, it will not be
// successful either.
unsafe impl PinCurrentCpu for DisabledLocalIrqGuard {}
// SAFETY: When preemption is disabled, the task cannot be preempted and migrates
// to another CPU.
unsafe impl PinCurrentCpu for DisabledPreemptGuard {}

/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The number of available processors is available.
/// 3. No CPU-local objects have been accessed.
pub(crate) unsafe fn init_on_bsp() {
    let num_cpus = crate::arch::boot::smp::get_num_processors().unwrap_or(1);

    // SAFETY: The safety is upheld by the caller and
    // the correctness of the `get_num_processors` method.
    unsafe {
        local::copy_bsp_for_ap(num_cpus as usize);

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
    unsafe {
        // FIXME: This is a global invariant,
        // better set before entering `ap_early_entry'.
        set_this_cpu_id(cpu_id);
    }
}
