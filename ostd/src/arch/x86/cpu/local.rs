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

use crate::cpu::local::SingleInstructionOps;

/// The GDT ensures that the FS segment is initialized to zero on boot.
/// This assertion checks that the base address has been set.
macro_rules! debug_assert_initialized {
    () => {
        // The compiler may think that [`super::get_base`] has side effects
        // so it may not be optimized out. We make sure that it will be
        // conditionally compiled only in debug builds.
        #[cfg(debug_assertions)]
        debug_assert_ne!(get_base(), 0);
    };
}

macro_rules! impl_single_instruction_for {
    ($([$typ: ty, $register_format: expr]),*) => {$(

        impl SingleInstructionOps for $typ {
            type Rhs = $typ;

            unsafe fn cpu_local_add_assign(offset: *mut Self, val: Self::Rhs) {
                debug_assert_initialized!();

                core::arch::asm!(
                    concat!("add fs:[{0}], {1:", $register_format, "}"),
                    in(reg) offset,
                    in(reg) val,
                    options(nostack),
                );
            }

            unsafe fn cpu_local_sub_assign(offset: *mut Self, val: Self::Rhs) {
                debug_assert_initialized!();

                core::arch::asm!(
                    concat!("sub fs:[{0}], {1:", $register_format, "}"),
                    in(reg) offset,
                    in(reg) val,
                    options(nostack),
                );
            }

            unsafe fn cpu_local_read(offset: *const Self) -> Self {
                debug_assert_initialized!();

                let val: Self;
                core::arch::asm!(
                    concat!("mov {0:", $register_format, "}, fs:[{1}]"),
                    out(reg) val,
                    in(reg) offset,
                    options(nostack, readonly),
                );
                val
            }

            unsafe fn cpu_local_write(offset: *mut Self, val: Self) {
                debug_assert_initialized!();

                core::arch::asm!(
                    concat!("mov fs:[{0}], {1:", $register_format, "}"),
                    in(reg) offset,
                    in(reg) val,
                    options(nostack),
                );
            }
        }

    )*};
}

impl_single_instruction_for!([u64, "r"], [usize, "r"], [u32, "e"], [u16, "x"]);
