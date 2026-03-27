// SPDX-License-Identifier: MPL-2.0

//! I/O memory utilities.

use crate::{arch::mm::__memcpy_fallible, io::io_mem::chunk::for_aligned_chunks, mm::PodOnce};

/// Reads from a pointer with a non-tearing memory load.
///
/// # Safety
/// Same as the safety requirement of `core::ptr::read_volatile`.
///
/// # Guarantee
///
/// The single-memory-load-instruction guarantee is particularly useful for
/// Confidential VMs (CVMs),
/// where the memory load may cause CPU exceptions
/// and the kernel has to handle such exceptions
/// by decoding the faulting CPU instruction.
/// As such, the kernel must be compiled to emit simple load/store CPU instructions.
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    // TODO: Use arch-specific single-instruction load for RISC-V.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    unsafe { core::ptr::read_volatile(ptr) }
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
    // TODO: Use arch-specific single-instruction store for RISC-V.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    unsafe { core::ptr::write_volatile(ptr, val) }
}

/// Copies from MMIO to regular memory.
///
/// # Safety
///
/// - `dst_ptr` must be valid for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio(
    mut dst_ptr: *mut u8,
    mut src_io_ptr: *const u8,
    mut count: usize,
) {
    // TODO: Optimize the copy loop to use larger access widths (e.g., u64)
    // and implement proper memory barriers for RISC-V.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    while count >= 1 {
        // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
        unsafe {
            let val: u8 = read_once(src_io_ptr);
            core::ptr::write(dst_ptr, val);
            src_io_ptr = src_io_ptr.add(1);
            dst_ptr = dst_ptr.add(1);
        }
        count -= 1;
    }
}

/// Copies from regular memory to MMIO.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio(
    mut src_ptr: *const u8,
    mut dst_io_ptr: *mut u8,
    mut count: usize,
) {
    // TODO: Optimize the copy loop to use larger access widths (e.g., u64)
    // and implement proper memory barriers for RISC-V.
    // For detail, see https://github.com/asterinas/asterinas/issues/2948.
    while count >= 1 {
        // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
        unsafe {
            let val: u8 = core::ptr::read(src_ptr);
            write_once(dst_io_ptr, val);
            src_ptr = src_ptr.add(1);
            dst_io_ptr = dst_io_ptr.add(1);
        }
        count -= 1;
    }
}

/// Copies from MMIO to MMIO.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_mmio_to_mmio(
    mut dst_io_ptr: *mut u8,
    mut src_io_ptr: *const u8,
    count: usize,
) {
    // TODO: Implement an efficient mmio to mmio copy using architecture-specific assembly instructions.
    // SAFETY: The caller guarantees source/destination validity for `count` bytes,
    // and `for_aligned_chunks!` only selects chunk sizes compatible with `dst_io_ptr` alignment.
    let copied = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        1,
        copy_mmio_to_mmio_val(&mut dst_io_ptr, &mut src_io_ptr)
    );
    debug_assert_eq!(copied, count);
}

/// Fills MMIO with `value` for `count` bytes.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn memset_mmio(mut dst_io_ptr: *mut u8, value: u8, count: usize) {
    // TODO: Implement an efficient memset using architecture-specific assembly instructions.
    // SAFETY: The caller guarantees source/destination validity for `count` bytes,
    // and `for_aligned_chunks!` only selects chunk sizes compatible with `dst_io_ptr` alignment.
    let written = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        1,
        memset_mmio_val(&mut dst_io_ptr, value)
    );
    debug_assert_eq!(written, count);
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
    // SAFETY: The caller guarantees the source and destination are valid.
    let failed_bytes = unsafe { __memcpy_fallible(dst_io_ptr, src_ptr, count) };
    count - failed_bytes
}

/// Copies one typed value from MMIO source to MMIO destination.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `size_of::<T>()` bytes.
/// - Both pointers must be aligned to `align_of::<T>()`.
unsafe fn copy_mmio_to_mmio_val<T: PodOnce>(
    dst_io_ptr: &mut *mut u8,
    src_io_ptr: &mut *const u8,
) -> usize {
    debug_assert!((*dst_io_ptr).addr().is_multiple_of(align_of::<T>()));
    debug_assert!((*src_io_ptr).addr().is_multiple_of(align_of::<T>()));
    // SAFETY: The caller guarantees both pointers are valid and aligned for `T`.
    unsafe {
        let val: T = read_once((*src_io_ptr).cast::<T>());
        write_once((*dst_io_ptr).cast::<T>(), val);
    }
    *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    *src_io_ptr = (*src_io_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}

/// Writes one typed repeated-byte value into MMIO destination.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes.
/// - `dst_io_ptr` must be aligned to `align_of::<T>()`.
unsafe fn memset_mmio_val<T: PodOnce>(dst_io_ptr: &mut *mut u8, value: u8) -> usize {
    debug_assert!((*dst_io_ptr).addr().is_multiple_of(align_of::<T>()));

    let repeated = u64::from_ne_bytes([value; 8]);
    // SAFETY: The caller guarantees destination pointer is valid and aligned for `T`.
    unsafe {
        match size_of::<T>() {
            1 => write_once((*dst_io_ptr).cast::<u8>(), repeated as u8),
            2 => write_once((*dst_io_ptr).cast::<u16>(), repeated as u16),
            4 => write_once((*dst_io_ptr).cast::<u32>(), repeated as u32),
            8 => write_once((*dst_io_ptr).cast::<u64>(), repeated),
            _ => core::hint::unreachable_unchecked(),
        }
    }

    *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}
