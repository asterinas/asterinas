// SPDX-License-Identifier: MPL-2.0

use crate::mm::PodOnce;

#[inline(always)]
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    // TODO: Use arch-specific single-instruction load for LoongArch.
    unsafe { core::ptr::read_volatile(ptr) }
}

#[inline(always)]
pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    // TODO: Use arch-specific single-instruction store for LoongArch.
    unsafe { core::ptr::write_volatile(ptr, val) }
}
