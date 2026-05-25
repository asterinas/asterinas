// SPDX-License-Identifier: MPL-2.0

//! I/O memory utilities.

use crate::{
    arch::mm::__memcpy_fallible,
    io::io_mem::util::{copy_from_mmio_val, copy_to_mmio_val, for_aligned_chunks, memset_mmio_val},
    mm::PodOnce,
};

/// Reads from a pointer with a non-tearing memory load.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::read_volatile`.
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    // SAFETY: The caller upholds the preconditions of `read_volatile`.
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Writes to a pointer with a non-tearing memory store.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::write_volatile`.
pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    // SAFETY: The caller upholds the preconditions of `write_volatile`.
    unsafe { core::ptr::write_volatile(ptr, val) }
}

/// Copies from MMIO to regular memory.
///
/// # Safety
///
/// - `dst_ptr` must be valid for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio(mut dst_ptr: *mut u8, mut src_io_ptr: *const u8, count: usize) {
    // SAFETY: The caller guarantees source/destination validity for `count` bytes,
    // and `for_aligned_chunks!` only selects chunk sizes compatible with `src_io_ptr` alignment.
    let copied = for_aligned_chunks!(
        src_io_ptr.addr(),
        count,
        8,
        copy_from_mmio_val(&mut dst_ptr, &mut src_io_ptr)
    );
    debug_assert_eq!(copied, count);
}

/// Copies from regular memory to MMIO.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio(mut src_ptr: *const u8, mut dst_io_ptr: *mut u8, count: usize) {
    // SAFETY: The caller guarantees source/destination validity for `count` bytes,
    // and `for_aligned_chunks!` only selects chunk sizes compatible with `dst_io_ptr` alignment.
    let copied = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        8,
        copy_to_mmio_val(&mut src_ptr, &mut dst_io_ptr)
    );
    debug_assert_eq!(copied, count);
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
    let failed_bytes = unsafe { __memcpy_fallible(dst_io_ptr, src_ptr, count) };
    count - failed_bytes
}

/// Fills MMIO with `value` for `count` bytes.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn memset_mmio(mut dst_io_ptr: *mut u8, value: u8, count: usize) {
    let written = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        8,
        memset_mmio_val(&mut dst_io_ptr, value)
    );
    debug_assert_eq!(written, count);
}
