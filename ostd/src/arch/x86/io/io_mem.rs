// SPDX-License-Identifier: MPL-2.0

//! I/O memory utilities.

use core::arch::asm;

use super::cvm;
use crate::{
    arch::{if_tdx_enabled, mm::__memcpy_fallible},
    mm::PodOnce,
};

/// Reads from a pointer with a non-tearing memory load.
///
/// This function is semantically equivalent to `core::ptr::read_volatile`
/// but is manually implemented with a single memory load instruction.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::read_volatile`.
///
/// # Guarantee
///
/// The single-memory-load-instruction guarantee is particularly useful for
/// Confidential VMs (CVMs) such as Intel TDX and AMD SEV,
/// where memory loads may cause CPU exceptions (#VE and #VC, respectively)
/// and the kernel has to handle such exceptions
/// by decoding the faulting CPU instruction.
/// As such, the kernel must be compiled to emit simple load/store CPU instructions.
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    debug_assert!(ptr.is_aligned());
    let mut val: u64 = 0;

    // SAFETY: The caller guarantees `ptr` is valid for reads.
    unsafe {
        // EFFICIENCY: The match should be optimized out for release builds.
        //
        // This match is resolved at compile-time via monomorphization.
        // Since `size_of::<T>()` is a constant for each concrete instance of this
        // function, the compiler eliminates the branch and only emits the
        // `MOV` instruction for the matching size.
        match size_of::<T>() {
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
        // EFFICIENCY: This should compile to a no-op for release builds.
        // This is because both the source and destination locations fit in a register.
        // So it only _re-interprets_ bits and no copying is needed.
        core::ptr::read((&val as *const u64).cast::<T>())
    }
}

/// Writes to a pointer with a non-tearing memory store.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::write_volatile`.
///
/// # Guarantee
///
/// Refer to the "Guarantee" section of [`read_once`].
pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    debug_assert!(ptr.is_aligned());
    let mut tmp: u64 = 0;

    // SAFETY: The caller guarantees `ptr` is valid for writes.
    unsafe {
        // EFFICIENCY: This should be a no-op for release build.
        // This is because both the source and destination locations fit in a register.
        // So it only _re-interprets_ bits and no copying is needed.
        core::ptr::write((&mut tmp as *mut u64).cast::<T>(), val);

        // EFFICIENCY: The match here has no overhead for release build.
        //
        // This match is resolved at compile-time via monomorphization.
        // Since `size_of::<T>()` is a constant for each concrete instance of this
        // function, the compiler eliminates the branch and only emits the
        // `MOV` instruction for the matching size.
        match size_of::<T>() {
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

macro_rules! do_copy_mmio_impl {
    (dst: $dst:ident, src: $src:ident, count: $count:ident) => {{
        if $count == 0 { return; }

        // SAFETY: The caller guarantees source and destination are valid for `count` bytes.
        unsafe {
            asm!(
                "rep movsb",
                inout("rdi") $dst => _,
                inout("rsi") $src => _,
                inout("rcx") $count => _,
                options(nostack, preserves_flags)
            );
        }
    }};
}

macro_rules! do_memset_mmio_impl {
    (dst: $dst:ident, value: $value:ident, count: $count:ident) => {{
        if $count == 0 { return; }

        // SAFETY: The caller guarantees destination pointer is valid for `count` bytes.
        unsafe {
            asm!(
                "rep stosb",
                inout("rdi") $dst => _,
                inout("rcx") $count => _,
                in("al") $value,
                options(nostack, preserves_flags)
            );
        }
    }};
}

/// Copies from MMIO to regular memory.
///
/// # Safety
///
/// - `dst_ptr` must be valid for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio(dst_ptr: *mut u8, src_io_ptr: *const u8, count: usize) {
    if_tdx_enabled!({
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        return unsafe { cvm::copy_from_mmio(dst_ptr, src_io_ptr, count) };
    });

    do_copy_mmio_impl!(dst: dst_ptr, src: src_io_ptr, count: count);
}

/// Copies from regular memory to MMIO.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio(src_ptr: *const u8, dst_io_ptr: *mut u8, count: usize) {
    if_tdx_enabled!({
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        return unsafe { cvm::copy_to_mmio(src_ptr, dst_io_ptr, count) };
    });

    do_copy_mmio_impl!(dst: dst_io_ptr, src: src_ptr, count: count);
}

/// Copies from MMIO to fallible memory and returns bytes copied.
///
/// # Safety
///
/// - `dst_ptr` must be valid or in user space for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio_fallible(
    dst_ptr: *mut u8,
    src_io_ptr: *const u8,
    count: usize,
) -> usize {
    if_tdx_enabled!({
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        return unsafe { cvm::copy_from_mmio_fallible(dst_ptr, src_io_ptr, count) };
    });

    // SAFETY: The caller guarantees the source and destination are valid.
    let failed_bytes = unsafe { __memcpy_fallible(dst_ptr, src_io_ptr, count) };
    count - failed_bytes
}

/// Copies from fallible memory to MMIO and returns bytes copied.
///
/// # Safety
///
/// - `src_ptr` must be valid or in user space for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio_fallible(
    src_ptr: *const u8,
    dst_io_ptr: *mut u8,
    count: usize,
) -> usize {
    if_tdx_enabled!({
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        return unsafe { cvm::copy_to_mmio_fallible(src_ptr, dst_io_ptr, count) };
    });

    // SAFETY: The caller guarantees the source and destination are valid.
    let failed_bytes = unsafe { __memcpy_fallible(dst_io_ptr, src_ptr, count) };
    count - failed_bytes
}

/// Fills MMIO with `value` for `count` bytes.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn memset_mmio(dst_io_ptr: *mut u8, value: u8, count: usize) {
    if_tdx_enabled!({
        // SAFETY: The caller guarantees the destination pointer is valid for `count` bytes.
        return unsafe { cvm::memset_mmio(dst_io_ptr, value, count) };
    });

    do_memset_mmio_impl!(dst: dst_io_ptr, value: value, count: count);
}
