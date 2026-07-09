// SPDX-License-Identifier: MPL-2.0

//! Fallible memory operations that can recover from page faults.
//!
//! TODO: These currently perform ordinary accesses without exception-table
//! recovery. Once the AArch64 exception vectors dispatch user page faults
//! through the exception table (see [`crate::arch::trap`]), reimplement these
//! in assembly with `.ex_table` entries, mirroring the RISC-V port.

use core::sync::atomic::{AtomicU32, Ordering};

/// Copies `size` bytes from `src` to `dst`, returning the number of bytes that
/// failed to copy.
///
/// # Safety
///
/// `src` must be valid for reads and `dst` valid for writes of `size` bytes.
pub(crate) unsafe fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize {
    // SAFETY: The caller guarantees both regions are valid for `size` bytes.
    unsafe { core::ptr::copy(src, dst, size) };
    0
}

/// Fills `size` bytes at `dst` with `value`, returning the number of bytes that
/// failed to set.
///
/// # Safety
///
/// `dst` must be valid for writes of `size` bytes.
pub(crate) unsafe fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize {
    // SAFETY: The caller guarantees the region is valid for `size` bytes.
    unsafe { core::ptr::write_bytes(dst, value, size) };
    0
}

/// Atomically loads a 32-bit value, returning `!0u64` on failure.
///
/// # Safety
///
/// `ptr` must be valid for an atomic 32-bit read.
pub(crate) unsafe fn __atomic_load_fallible(ptr: *const u32) -> u64 {
    // SAFETY: The caller guarantees `ptr` is valid for an atomic load.
    let atomic = unsafe { AtomicU32::from_ptr(ptr as *mut u32) };
    atomic.load(Ordering::Acquire) as u64
}

/// Atomically compares and exchanges a 32-bit value, returning the previous
/// value, or `!0u64` on failure.
///
/// # Safety
///
/// `ptr` must be valid for an atomic 32-bit read-modify-write.
pub(crate) unsafe fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64 {
    // SAFETY: The caller guarantees `ptr` is valid for an atomic RMW.
    let atomic = unsafe { AtomicU32::from_ptr(ptr) };
    match atomic.compare_exchange(old_val, new_val, Ordering::AcqRel, Ordering::Acquire) {
        Ok(prev) => prev as u64,
        Err(prev) => prev as u64,
    }
}
