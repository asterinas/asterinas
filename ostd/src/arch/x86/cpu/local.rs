// SPDX-License-Identifier: MPL-2.0

//! Architecture dependent CPU-local information utilities.

use x86_64::registers::segmentation::{Segment64, FS};

/// Sets the base address for the CPU local storage by writing to the FS base model-specific register.
/// This operation is marked as `unsafe` because it directly interfaces with low-level CPU registers.
///
/// # Safety
///
///  - This function is safe to call provided that the FS register is dedicated entirely for CPU local storage
///    and is not concurrently accessed for other purposes.
///  - The caller must ensure that `addr` is a valid address and properly aligned, as required by the CPU.
///  - This function should only be called in contexts where the CPU is in a state to accept such changes,
///    such as during processor initialization.
pub(crate) unsafe fn set_base(addr: u64) {
    FS::write_base(x86_64::addr::VirtAddr::new(addr));
}

/// Gets the base address for the CPU local storage by reading the FS base model-specific register.
pub(crate) fn get_base() -> u64 {
    FS::read_base().as_u64()
}

pub mod preempt_lock_count {
    //! We need to increment/decrement the per-CPU preemption lock count using
    //! a single instruction. This requirement is stated by
    //! [`crate::task::processor::PreemptInfo`].

    /// The GDT ensures that the FS segment is initialized to zero on boot.
    /// This assertion checks that the base address has been set.
    macro_rules! debug_assert_initialized {
        () => {
            // The compiler may think that [`super::get_base`] has side effects
            // so it may not be optimized out. We make sure that it will be
            // conditionally compiled only in debug builds.
            #[cfg(debug_assertions)]
            debug_assert_ne!(super::get_base(), 0);
        };
    }

    /// Increments the per-CPU preemption lock count using one instruction.
    pub(crate) fn inc() {
        debug_assert_initialized!();

        // SAFETY: The inline assembly increments the lock count in one
        // instruction without side effects.
        unsafe {
            core::arch::asm!(
                "add dword ptr fs:[__cpu_local_preempt_lock_count], 1",
                options(nostack),
            );
        }
    }

    /// Decrements the per-CPU preemption lock count using one instruction.
    pub(crate) fn dec() {
        debug_assert_initialized!();

        // SAFETY: The inline assembly decrements the lock count in one
        // instruction without side effects.
        unsafe {
            core::arch::asm!(
                "sub dword ptr fs:[__cpu_local_preempt_lock_count], 1",
                options(nostack),
            );
        }
    }

    /// Gets the per-CPU preemption lock count using one instruction.
    pub(crate) fn get() -> u32 {
        debug_assert_initialized!();

        let count: u32;
        // SAFETY: The inline assembly reads the lock count in one instruction
        // without side effects.
        unsafe {
            core::arch::asm!(
                "mov {0:e}, fs:[__cpu_local_preempt_lock_count]",
                out(reg) count,
                options(nostack, readonly),
            );
        }
        count
    }
}
