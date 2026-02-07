// SPDX-License-Identifier: MPL-2.0

use core::arch::asm;

use crate::mm::PodOnce;

#[inline(always)]
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    let mut val: u64 = 0;
    let size = core::mem::size_of::<T>();

    unsafe {
        match size {
            1 => {
                asm!("mov {0:l}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            2 => {
                asm!("mov {0:x}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            4 => {
                asm!("mov {0:e}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            8 => {
                asm!("mov {0}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
        core::ptr::read_unaligned(&val as *const u64 as *const T)
    }
}

#[inline(always)]
pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    let mut tmp: u64 = 0;

    unsafe {
        core::ptr::write_unaligned(&mut tmp as *mut u64 as *mut T, val);

        match core::mem::size_of::<T>() {
            1 => {
                asm!("mov [{0}], {1:l}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            2 => {
                asm!("mov [{0}], {1:x}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            4 => {
                asm!("mov [{0}], {1:e}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            8 => {
                asm!("mov [{0}], {1}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
    }
}
