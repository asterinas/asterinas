// SPDX-License-Identifier: MPL-2.0

//! Alignment-aware utilities for MMIO copy and fill operations.

use crate::{
    arch::io::io_mem::{read_once, write_once},
    mm::PodOnce,
};

/// Iterates over aligned chunk sizes and dispatches to typed MMIO operations.
///
/// This macro intentionally keeps the required `unsafe` operation centralized,
/// so call sites only need to justify one high-level safety argument.
///
/// # Arguments
///
/// - `$io_addr`: Base I/O address used to derive alignment for each chunk.
/// - `$count`: Total bytes to process.
/// - `$max_chunk`: Maximum chunk width in bytes. Must be a power of two in `1..=8`.
/// - `$op($args...)`: Typed operation called as `$op::<T>($args...)` for each selected chunk,
///   where `T` is one of `u8`, `u16`, `u32`, or `u64`.
///
/// # Safety
///
/// - The enclosing function must ensure that each invocation of `$op::<T>(...)`
///   is valid for every chunk type selected by this macro (`u8`, `u16`, `u32`, `u64`).
/// - In particular, pointer validity and alignment for the selected chunk sizes
///   must hold for the full `$count` range.
/// - For MMIO, the caller must also ensure the target range supports accesses
///   at all selected widths. Some devices require fixed-width accesses and may
///   fault or behave unexpectedly when split into smaller operations.
/// - `$op` must advance any pointer arguments consistently with the returned
///   byte count so that chunk iteration stays synchronized.
macro_rules! for_aligned_chunks {
    ($io_addr:expr, $count:expr, $max_chunk:expr, $op:ident ( $($arg:expr),* $(,)? )) => {{
        crate::io::io_mem::util::for_each_chunk_with_max($io_addr, $count, $max_chunk, |chunk_size| {
            match chunk_size {
                1 => {
                    // SAFETY: The caller of the enclosing function guarantees pointer validity
                    // and alignment requirements for this chunk size.
                    unsafe { $op::<u8>($($arg),*) }
                }
                2 => {
                    // SAFETY: The caller of the enclosing function guarantees pointer validity
                    // and alignment requirements for this chunk size.
                    unsafe { $op::<u16>($($arg),*) }
                }
                4 => {
                    // SAFETY: The caller of the enclosing function guarantees pointer validity
                    // and alignment requirements for this chunk size.
                    unsafe { $op::<u32>($($arg),*) }
                }
                8 => {
                    // SAFETY: The caller of the enclosing function guarantees pointer validity
                    // and alignment requirements for this chunk size.
                    unsafe { $op::<u64>($($arg),*) }
                }
                _ => unreachable!("unexpected chunk size: {}", chunk_size),
            }
        })
    }};
}

pub(crate) use for_aligned_chunks;

#[doc(hidden)]
pub(crate) fn for_each_chunk_with_max(
    mut io_addr: usize,
    count: usize,
    max_chunk: usize,
    mut copy_chunk_fn: impl FnMut(usize) -> usize,
) -> usize {
    debug_assert!(max_chunk.is_power_of_two());
    debug_assert!((1..=8).contains(&max_chunk));

    let mut copied_total = 0;
    let mut remaining = count;
    let max_log2 = max_chunk.trailing_zeros();

    while remaining > 0 {
        let addr_align_log2 = io_addr.trailing_zeros();
        let mut chunk_size = 1 << addr_align_log2.min(max_log2);

        if chunk_size > remaining {
            chunk_size = 1 << (usize::BITS - 1 - remaining.leading_zeros());
        }

        let copied = copy_chunk_fn(chunk_size);
        debug_assert!(copied <= chunk_size);
        copied_total += copied;
        if copied < chunk_size {
            return copied_total;
        }

        remaining -= chunk_size;
        io_addr += chunk_size;
    }

    copied_total
}

/// Copies from MMIO to regular memory for a single value of type `T`.
///
/// # Safety
///
/// - `src_io_ptr` must be valid for MMIO reads of `size_of::<T>()` bytes and properly aligned.
/// - `dst_ptr` must be valid for writes of `size_of::<T>()` bytes.
pub(crate) unsafe fn copy_from_mmio_val<T: PodOnce>(
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
    }

    *src_io_ptr = (*src_io_ptr).wrapping_add(size_of::<T>());
    *dst_ptr = (*dst_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}

/// Copies from regular memory to MMIO for a single value of type `T`.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `size_of::<T>()` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes and
///   must be aligned to `align_of::<T>()`.
pub(crate) unsafe fn copy_to_mmio_val<T: PodOnce>(
    src_ptr: &mut *const u8,
    dst_io_ptr: &mut *mut u8,
) -> usize {
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
    }

    *src_ptr = (*src_ptr).wrapping_add(size_of::<T>());
    *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}

/// Writes one typed repeated-byte value into MMIO destination.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `size_of::<T>()` bytes and aligned to `align_of::<T>()`.
pub(crate) unsafe fn memset_mmio_val<T: PodOnce>(dst_io_ptr: &mut *mut u8, value: u8) -> usize {
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
