// SPDX-License-Identifier: MPL-2.0

//! CPU-related definitions.

pub mod local;
pub mod set;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        pub use crate::arch::x86::cpu::*;
    } else if #[cfg(target_arch = "riscv64")] {
        pub use crate::arch::riscv::cpu::*;
    }
}

pub use set::{AtomicCpuSet, CpuSet};
use spin::Once;

use crate::{
    arch::boot::smp::get_num_processors, cpu_local_cell, task::DisabledPreemptGuard,
    trap::DisabledLocalIrqGuard,
};

/// The number of CPUs.
static NUM_CPUS: Once<u32> = Once::new();

/// Initializes the number of CPUs.
///
/// # Safety
///
/// The caller must ensure that this function is called only once on the BSP
/// at the correct time when the number of CPUs is available from the platform.
pub(crate) unsafe fn init_num_cpus() {
    let num_processors = get_num_processors().unwrap_or(1);
    NUM_CPUS.call_once(|| num_processors);
}

/// Initializes the number of the current CPU.
///
/// # Safety
///
/// The caller must ensure that this function is called only once on the
/// correct CPU with the correct CPU ID.
pub(crate) unsafe fn set_this_cpu_id(id: u32) {
    CURRENT_CPU.store(id);
}

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    debug_assert!(
        NUM_CPUS.get().is_some(),
        "The number of CPUs is not initialized"
    );
    // SAFETY: The number of CPUs is initialized. The unsafe version is used
    // to avoid the overhead of the check.
    unsafe { *NUM_CPUS.get_unchecked() }
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
    fn current_cpu(&self) -> u32 {
        let id = CURRENT_CPU.load();
        debug_assert_ne!(id, u32::MAX, "This CPU is not initialized");
        id
    }
}

// SAFETY: When IRQs are disabled, the task cannot be passively preempted and
// migrates to another CPU. If the task actively calls `yield`, it will not be
// successful either.
unsafe impl PinCurrentCpu for DisabledLocalIrqGuard {}
// SAFETY: When preemption is disabled, the task cannot be preempted and migrates
// to another CPU.
unsafe impl PinCurrentCpu for DisabledPreemptGuard {}

cpu_local_cell! {
    /// The number of the current CPU.
    static CURRENT_CPU: u32 = u32::MAX;
}
