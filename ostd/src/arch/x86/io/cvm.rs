// SPDX-License-Identifier: MPL-2.0

//! I/O memory utilities for confidential VMs.

use super::io_mem::{read_once, write_once};
use crate::{arch::mm::__memcpy_fallible, io::io_mem::chunk::for_aligned_chunks, mm::PodOnce};

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

/// Copies from MMIO to MMIO with CVM-safe access granularity.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(super) unsafe fn copy_mmio_to_mmio(
    mut dst_io_ptr: *mut u8,
    mut src_io_ptr: *const u8,
    count: usize,
) {
    if count == 0 {
        return;
    }
    let diff = dst_io_ptr.addr() ^ src_io_ptr.addr();
    let max_align = if diff == 0 {
        8
    } else {
        1 << diff.trailing_zeros().min(3)
    };

    // SAFETY: The caller guarantees both MMIO pointers are valid for `count` bytes,
    // and `for_aligned_chunks!` only selects chunk sizes compatible with `dst_io_ptr` alignment.
    let copied = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        max_align,
        copy_mmio_to_mmio_val(&mut dst_io_ptr, &mut src_io_ptr)
    );
    debug_assert_eq!(copied, count);
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
    let size = size_of::<T>();
    let mut copied = 0;

    // Keep MMIO side effects aligned with committed bytes in fallible destination memory.
    while copied < size {
        // SAFETY: The caller ensures MMIO source range is valid for reads.
        let byte = unsafe { read_once::<u8>((*src_io_ptr).wrapping_add(copied)) };
        // SAFETY: The caller ensures destination range is valid or in user space for writes.
        let failed = unsafe { __memcpy_fallible((*dst_ptr).wrapping_add(copied), &byte, 1) };
        if failed != 0 {
            break;
        }
        copied += 1;
    }

    *dst_ptr = (*dst_ptr).wrapping_add(copied);
    *src_io_ptr = (*src_io_ptr).wrapping_add(copied);
    copied
}

/// Copies from MMIO to regular memory for a single value of type `T`.
///
/// # Safety
///
/// - `src_io_ptr` must be valid for MMIO reads of `size_of::<T>()` bytes and properly aligned.
/// - `dst_ptr` must be valid for writes of `size_of::<T>()` bytes.
unsafe fn copy_from_mmio_val<T: PodOnce>(
    dst_ptr: &mut *mut u8,
    src_io_ptr: &mut *const u8,
) -> usize {
    debug_assert!((*src_io_ptr).addr().is_multiple_of(align_of::<T>()));
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe {
        let val: T = read_once((*src_io_ptr).cast::<T>());
        let dst = *dst_ptr;
        let align_mask = size_of::<T>() - 1;
        if dst.addr() & align_mask == 0 {
            core::ptr::write(dst.cast::<T>(), val);
        } else {
            core::ptr::write_unaligned(dst.cast::<T>(), val);
        }
        *src_io_ptr = (*src_io_ptr).wrapping_add(size_of::<T>());
        *dst_ptr = dst.wrapping_add(size_of::<T>());
    }
    size_of::<T>()
}

/// Copies from regular memory to MMIO for a single value of type `T`.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `size_of::<T>()` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes and
///   must be aligned to `align_of::<T>()`.
unsafe fn copy_to_mmio_val<T: PodOnce>(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) -> usize {
    debug_assert!((*dst_io_ptr).addr().is_multiple_of(align_of::<T>()));
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe {
        let src = *src_ptr;
        let align_mask = size_of::<T>() - 1;
        let val: T = if src.addr() & align_mask != 0 {
            core::ptr::read_unaligned(src.cast::<T>())
        } else {
            core::ptr::read(src.cast::<T>())
        };
        write_once((*dst_io_ptr).cast::<T>(), val);
        *src_ptr = src.wrapping_add(size_of::<T>());
        *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    }
    size_of::<T>()
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
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes and aligned to `align_of::<T>()`.
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
