// SPDX-License-Identifier: MPL-2.0

//! I/O memory utilities for confidential VMs.

use super::io_mem::read_once;
use crate::{
    arch::mm::__memcpy_fallible,
    io::io_mem::util::{copy_from_mmio_val, copy_to_mmio_val, for_aligned_chunks, memset_mmio_val},
    mm::PodOnce,
};

/// Copies from MMIO to regular memory with CVM-safe access granularity.
///
/// # Safety
///
/// - `dst_ptr` must be valid for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(super) unsafe fn copy_from_mmio(mut dst_ptr: *mut u8, mut src_io_ptr: *const u8, count: usize) {
    if count == 0 {
        return;
    }

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

/// Copies from regular memory to MMIO with CVM-safe access granularity.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(super) unsafe fn copy_to_mmio(mut src_ptr: *const u8, mut dst_io_ptr: *mut u8, count: usize) {
    if count == 0 {
        return;
    }

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

/// Copies from MMIO to fallible memory with CVM-safe access granularity.
///
/// # Safety
///
/// - `dst_ptr` must be valid or in user space for writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(super) unsafe fn copy_from_mmio_fallible(
    mut dst_ptr: *mut u8,
    mut src_io_ptr: *const u8,
    count: usize,
) -> usize {
    // SAFETY: The caller guarantees MMIO source validity for `count` bytes.
    // The callee handles partial completion when the fallible destination faults.
    for_aligned_chunks!(
        src_io_ptr.addr(),
        count,
        8,
        copy_from_mmio_val_fallible(&mut dst_ptr, &mut src_io_ptr)
    )
}

/// Copies from fallible memory to MMIO with CVM-safe access granularity.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
/// - `src_ptr` must be valid or in user space for reads of `count` bytes.
pub(super) unsafe fn copy_to_mmio_fallible(
    mut src_ptr: *const u8,
    mut dst_io_ptr: *mut u8,
    mut count: usize,
) -> usize {
    const BUF_SIZE: usize = 256;
    let mut buf = [0u8; BUF_SIZE];
    let mut copied_total = 0;

    while count > 0 {
        let chunk = core::cmp::min(count, BUF_SIZE);

        // SAFETY: The caller ensures `src_ptr` is valid or in user space for reads.
        let failed = unsafe { __memcpy_fallible(buf.as_mut_ptr(), src_ptr, chunk) };
        let copied = chunk - failed;

        if copied < chunk {
            // For MMIO safety, avoid issuing a partial chunk write when source fault occurs.
            return copied_total;
        }

        // SAFETY: The caller ensures destination MMIO range is valid for `chunk` bytes.
        unsafe { copy_to_mmio(buf.as_ptr(), dst_io_ptr, chunk) };
        dst_io_ptr = dst_io_ptr.wrapping_add(chunk);
        src_ptr = src_ptr.wrapping_add(chunk);
        copied_total += chunk;
        count -= chunk;
    }

    copied_total
}

/// Fills MMIO with `value` for `count` bytes with CVM-safe access granularity.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(super) unsafe fn memset_mmio(mut dst_io_ptr: *mut u8, value: u8, count: usize) {
    if count == 0 {
        return;
    }

    // SAFETY: The caller guarantees destination MMIO pointer is valid for `count` bytes.
    let written = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        8,
        memset_mmio_val(&mut dst_io_ptr, value)
    );
    debug_assert_eq!(written, count);
}

/// Copies from MMIO to fallible memory for a single value and returns bytes copied.
///
/// # Safety
///
/// - `dst_ptr` must be valid or in user space for writes of `size_of::<T>()` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `size_of::<T>()` bytes.
unsafe fn copy_from_mmio_val_fallible<T: PodOnce>(
    dst_ptr: &mut *mut u8,
    src_io_ptr: &mut *const u8,
) -> usize {
    // SAFETY: The caller ensures MMIO source range is valid for reads.
    let value = unsafe { read_once::<T>((*src_io_ptr).cast::<T>()) };
    let size = size_of::<T>();
    // SAFETY: The caller ensures destination range is valid or in user space for writes.
    let failed =
        unsafe { __memcpy_fallible(*dst_ptr, core::ptr::addr_of!(value).cast::<u8>(), size) };
    let copied = size - failed;

    *dst_ptr = (*dst_ptr).wrapping_add(copied);
    *src_io_ptr = (*src_io_ptr).wrapping_add(copied);
    copied
}
