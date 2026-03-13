// SPDX-License-Identifier: MPL-2.0

use crate::mm::PodOnce;

pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    // TODO: Use arch-specific single-instruction load for LoongArch.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    unsafe { core::ptr::read_volatile(ptr) }
}

pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    // TODO: Use arch-specific single-instruction store for LoongArch.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    unsafe { core::ptr::write_volatile(ptr, val) }
}

/// Copies from MMIO to regular memory.
///
/// # Safety
///
/// - `dst` must be valid for writes of `count` bytes.
/// - `src` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio(mut dst: *mut u8, mut src: *const u8, mut count: usize) {
    // TODO: Optimize the copy loop to use larger access widths (e.g., u64)
    // and implement proper memory barriers for LoongArch.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    while count >= 1 {
        // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
        unsafe {
            let val: u8 = read_once(src);
            core::ptr::write(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }
}

/// Copies from regular memory to MMIO.
///
/// # Safety
///
/// - `src` must be valid for reads of `count` bytes.
/// - `dst` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio(mut src: *const u8, mut dst: *mut u8, mut count: usize) {
    // TODO: Optimize the copy loop to use larger access widths (e.g., u64)
    // and implement proper memory barriers for LoongArch.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    while count >= 1 {
        // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
        unsafe {
            let val: u8 = core::ptr::read(src);
            write_once(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }
}
