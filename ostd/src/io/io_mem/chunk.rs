// SPDX-License-Identifier: MPL-2.0

//! Shared chunk-iteration helpers for MMIO copy/fill routines.

/// Iterates over aligned chunk sizes and dispatches to typed MMIO operations.
///
/// This macro intentionally keeps the required `unsafe` operation centralized,
/// so call sites only need to justify one high-level safety argument.
///
/// # Safety
///
/// - The enclosing function must ensure that each invocation of `$op::<T>(...)`
///   is valid for every chunk type selected by this macro (`u8`, `u16`, `u32`, `u64`).
/// - In particular, pointer validity and alignment for the selected chunk sizes
///   must hold for the full `$count` range.
macro_rules! for_aligned_chunks {
    ($io_addr:expr, $count:expr, $max_chunk:expr, $op:ident ( $($arg:expr),* $(,)? )) => {{
        crate::io::io_mem::chunk::for_each_chunk_with_max($io_addr, $count, $max_chunk, |chunk_size| {
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
        copied_total += copied;
        if copied < chunk_size {
            return copied_total;
        }

        remaining -= chunk_size;
        io_addr += chunk_size;
    }

    copied_total
}
