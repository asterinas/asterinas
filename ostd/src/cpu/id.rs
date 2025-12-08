// SPDX-License-Identifier: MPL-2.0

//! CPU identification numbers.

pub use current::PinCurrentCpu;
pub use set::{AtomicCpuSet, CpuSet};

use crate::util::id_set::Id;

/// The ID of a CPU in the system.
///
/// If converting from/to an integer, the integer must start from 0 and be less
/// than the number of CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuId(u32);

impl CpuId {
    /// Creates a new instance.
    ///
    /// # Panics
    ///
    /// The given number must be smaller than the total number of CPUs
    /// (`ostd::cpu::num_cpus()`).
    pub fn new(raw_id: u32) -> Self {
        assert!(raw_id < num_cpus() as u32);
        // SAFETY: The raw ID is smaller than `num_cpus()`.
        unsafe { Self::new_unchecked(raw_id) }
    }

    /// Returns the CPU ID of the bootstrap processor (BSP).
    ///
    /// The number for the BSP is always zero.
    pub const fn bsp() -> Self {
        // BSP's `CURRENT_CPU` is assigned a value of 0.
        let bsp_raw_cpu_id = 0;
        // SAFETY: There is at least one CPU.
        Self(bsp_raw_cpu_id)
    }
}

impl From<CpuId> for u32 {
    fn from(cpu_id: CpuId) -> Self {
        cpu_id.0
    }
}

/// The error type returned when converting an out-of-range integer to [`CpuId`].
#[derive(Debug, Clone, Copy)]
pub struct CpuIdFromIntError;

impl TryFrom<usize> for CpuId {
    type Error = CpuIdFromIntError;

    fn try_from(raw_id: usize) -> Result<Self, Self::Error> {
        if raw_id < num_cpus() {
            // SAFETY: The raw ID is smaller than `num_cpus()`.
            let new_self = unsafe { CpuId::new_unchecked(raw_id as u32) };
            Ok(new_self)
        } else {
            Err(CpuIdFromIntError)
        }
    }
}

/// Returns the number of CPUs.
pub fn num_cpus() -> usize {
    // SAFETY: As far as the safe APIs are concerned, `NUM_CPUS` is
    // read-only, so it is always valid to read.
    (unsafe { NUM_CPUS }) as usize
}

static mut NUM_CPUS: u32 = 1;

/// Returns an iterator over all CPUs.
pub fn all_cpus() -> impl Iterator<Item = CpuId> {
    (0..num_cpus()).map(|raw_id| {
        // SAFETY: The raw ID is smaller than `num_cpus()`.
        unsafe { CpuId::new_unchecked(raw_id as u32) }
    })
}

mod set {
    use super::{CpuId, num_cpus};
    use crate::util::id_set::{AtomicIdSet, Id, IdSet};

    /// A set of CPU IDs.
    pub type CpuSet = IdSet<CpuId>;

    /// A set of CPU IDs, with support for concurrent access.
    pub type AtomicCpuSet = AtomicIdSet<CpuId>;

    // SAFETY: `CpuId`s and the integers within 0 to `num_cpus` (exclusive)
    // have 1:1 mapping.
    unsafe impl Id for CpuId {
        unsafe fn new_unchecked(raw_id: u32) -> Self {
            Self(raw_id)
        }

        fn cardinality() -> u32 {
            num_cpus() as u32
        }
    }
}

mod current {
    //! The current CPU ID.

    use super::CpuId;
    use crate::{cpu_local_cell, task::atomic_mode::InAtomicMode, util::id_set::Id};

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
    /// [`DisabledLocalIrqGuard`]: crate::irq::DisabledLocalIrqGuard
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

    impl CpuId {
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

            let current_raw_id = CURRENT_CPU.load();
            // SAFETY: The CPU-local value is initialized to a correct one.
            unsafe { Self::new_unchecked(current_raw_id) }
        }
    }

    /// Initializes the module on the current CPU.
    ///
    /// Note that this method will use the current CPU's CPU-local storage.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// 1. This method is called on each CPU in the boot context.
    /// 2. The CPU ID for the current CPU is correct.
    pub(super) unsafe fn init_on_cpu(current_cpu_id: u32) {
        // FIXME: If there are safe APIs that rely on the correctness of
        // the CPU ID for soundness, we'd better make the CPU ID a global
        // invariant and initialize it before entering `ap_early_entry`.
        CURRENT_CPU.store(current_cpu_id);

        #[cfg(debug_assertions)]
        IS_CURRENT_CPU_INITED.store(true);
    }

    cpu_local_cell! {
        /// The current CPU ID.
        pub(super) static CURRENT_CPU: u32 = 0;
        /// The initialization state of the current CPU ID.
        #[cfg(debug_assertions)]
        pub(super) static IS_CURRENT_CPU_INITED: bool = false;
    }
}

/// Initializes the CPU ID module (the BSP part).
///
/// Note that this method will use the BSP's CPU-local storage.
///
/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The number of CPUs is correct.
pub(super) unsafe fn init_on_bsp(num_cpus: u32) {
    // SAFETY:
    // 1. We're in the boot context of the BSP.
    // 2. The CPU ID of BSP has a value of zero.
    unsafe { current::init_on_cpu(0) };

    // SAFETY:
    // 1. We're in the boot context of the BSP and APs have not yet booted.
    // 2. The number of CPUs is correct.
    unsafe { init_num_cpus(num_cpus) };
}

/// Initializes the number of CPUs.
///
/// The number of CPUs is a fixed value,
/// since we don't support CPU hot-plugging.
///
/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The number of CPUs is correct.
unsafe fn init_num_cpus(num_cpus: u32) {
    // Thanks to this assertion,
    // it's only legal to call this function to
    // increase the number of CPUs from one (the initial value)
    // to the actual number of CPUs.
    assert!(num_cpus >= 1);

    // SAFETY: It is safe to mutate this global variable because we
    // are in the boot context.
    unsafe { NUM_CPUS = num_cpus };
}

/// Initializes the CPU ID module (the AP part).
///
/// Note that this method will use the BSP's CPU-local storage.
/// This should be fine
/// because `crate::cpu::init_on_bsp` must have been invoked before APs boot.
///
/// # Safety
///
/// The caller must ensure that:
/// 1. We're in the boot context of an AP.
/// 2. The CPU ID of the AP is correct.
pub(super) unsafe fn init_on_ap(cpu_id: u32) {
    // SAFETY:
    // 1. We're in the boot context of the AP.
    // 2. The CPU ID for the AP is correct.
    unsafe { current::init_on_cpu(cpu_id) };
}
