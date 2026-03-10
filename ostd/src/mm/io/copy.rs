// SPDX-License-Identifier: MPL-2.0

//! Generalized memory copy and fill operations for different memory types.
//!
//! This module provides [`Memcpy`] and [`Memset`] traits that generalize
//! the classic C `memcpy` and `memset` for typed memory categories
//! ([`Infallible`], [`Fallible`]).
//!
//! # Examples
//!
//! Infallible kernel-to-kernel copy:
//!
//! ```ignore
//! // SAFETY: Both pointers are valid kernel memory for `len` bytes.
//! unsafe { memcpy::<Infallible, Infallible>(dst, src, len) };
//! ```
//!
//! Fallible copy from user space into kernel space:
//!
//! ```ignore
//! // SAFETY: `dst` is valid kernel memory; `src` is in user space.
//! let copied = unsafe { memcpy::<Infallible, Fallible>(dst, src, len) };
//! if copied < len {
//!     // A page fault occurred after `copied` bytes.
//! }
//! ```
//!
//! Fallible zero-fill of user memory:
//!
//! ```ignore
//! // SAFETY: `dst` is in user space for `len` bytes.
//! let filled = unsafe { memset::<Fallible>(dst, 0u8, len) };
//! if filled < len {
//!     // A page fault occurred after `filled` bytes.
//! }
//! ```

use super::{Fallible, Infallible};
use crate::arch::mm::{__memcpy_fallible, __memset_fallible};

/// Copies `len` bytes from `src` to `dst`.
///
/// This is a generalized `memcpy` that dispatches
/// to the appropriate implementation based on the memory types.
/// Infallible-to-infallible copies return `()`.
/// Copies involving fallible memory are fallible
/// and return the number of bytes successfully copied.
///
/// # Safety
///
/// - `src` must point to a virtual memory region of type `Src`
///   that allows memory reads.
/// - `dst` must point to a virtual memory region of type `Dst`
///   that allows memory writes.
pub(crate) unsafe fn memcpy<Dst: Memcpy<Src>, Src>(
    dst: *mut u8,
    src: *const u8,
    len: usize,
) -> Dst::Result {
    // SAFETY: The safety is upheld by the caller.
    unsafe { Dst::memcpy(dst, src, len) }
}

/// Fills `len` bytes of memory at `dst` with the specified `value`.
///
/// This is a generalized `memset` that dispatches
/// to the appropriate implementation based on the memory type.
/// Infallible memory fills return `()`.
/// Fallible memory fills return the number of bytes successfully set.
///
/// # Safety
///
/// - `dst` must point to a virtual memory region of type `Dst`
///   that allows memory writes.
pub(crate) unsafe fn memset<Dst: Memset>(dst: *mut u8, value: u8, len: usize) -> Dst::Result {
    // SAFETY: The safety is upheld by the caller.
    unsafe { Dst::memset(dst, value, len) }
}

/// Generalizes `memcpy` for different memory types.
///
/// The destination type is `Self` and the source type is `Src`,
/// following the `memcpy(dst, src, len)` argument convention.
///
/// When the source and destination virtual/physical memory regions overlap,
/// the copy may produce unexpected bytes in the destination.
/// The exact behavior is up to the implementation.
///
/// # Safety
///
/// Assuming the caller upholds the preconditions of [`Memcpy::memcpy`],
/// the memory copy implementation must not cause soundness problems.
/// The exact safety conditions depend on the type of the source and destination memory.
/// For example, when both sides are infallible memory,
/// the implementation must copy exactly `len` bytes from `src` to `dst`.
pub(crate) unsafe trait Memcpy<Src> {
    /// The result type of the copy operation.
    ///
    /// Infallible copies return `()`.
    /// Fallible copies return the number of bytes successfully copied
    /// as `usize`.
    type Result;

    /// Copies `len` bytes from `src` to `dst`.
    ///
    /// # Safety
    ///
    /// - `src` must point to a virtual memory region of type `Src`
    ///   that allows memory reads.
    /// - `dst` must point to a virtual memory region of type `Self`
    ///   that allows memory writes.
    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> Self::Result;
}

/// Generalizes `memset` for different memory types.
///
/// # Safety
///
/// The implementation must correctly fill `len` bytes at `dst` with `value`
/// when the caller upholds the preconditions of [`Memset::memset`].
pub(crate) unsafe trait Memset {
    /// The result type of the fill operation.
    type Result;

    /// Fills `len` bytes of memory at `dst` with the specified `value`.
    ///
    /// # Safety
    ///
    /// - `dst` must point to a virtual memory region of type `Self`
    ///   that allows memory writes.
    unsafe fn memset(dst: *mut u8, value: u8, len: usize) -> Self::Result;
}

// SAFETY: Delegates to `volatile_copy_memory`,
// which correctly copies bytes between valid kernel memory regions.
unsafe impl Memcpy<Infallible> for Infallible {
    type Result = ();

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
        // This method is implemented by calling `volatile_copy_memory`. Note that even with the
        // "volatile" keyword, data races are still considered undefined behavior (UB) in both the
        // Rust documentation and the C/C++ standards. In general, UB makes the behavior of the
        // entire program unpredictable, usually due to compiler optimizations that assume the
        // absence of UB. However, in this particular case, considering that the Linux kernel uses
        // the "volatile" keyword to implement `READ_ONCE` and `WRITE_ONCE`, the compiler is
        // extremely unlikely to break our code unless it also breaks the Linux kernel.
        //
        // For more details and future possibilities, see
        // <https://github.com/asterinas/asterinas/pull/1001#discussion_r1667317406>.

        // SAFETY: The safety is upheld by the caller.
        unsafe { core::intrinsics::volatile_copy_memory(dst, src, len) };
    }
}

// SAFETY: Delegates to `__memcpy_fallible`,
// which handles page faults when copying from fallible to infallible memory.
unsafe impl Memcpy<Fallible> for Infallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        // SAFETY: The safety is upheld by the caller.
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

// SAFETY: Delegates to `__memcpy_fallible`,
// which handles page faults when copying from infallible to fallible memory.
unsafe impl Memcpy<Infallible> for Fallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        // SAFETY: The safety is upheld by the caller.
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

// SAFETY: Delegates to `__memcpy_fallible`,
// which handles page faults when copying between fallible memory regions.
unsafe impl Memcpy<Fallible> for Fallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        // SAFETY: The safety is upheld by the caller.
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

// SAFETY: Delegates to `volatile_set_memory`,
// which correctly fills bytes in valid kernel memory.
unsafe impl Memset for Infallible {
    type Result = ();

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) {
        // SAFETY: The safety is upheld by the caller.
        unsafe { core::intrinsics::volatile_set_memory(dst, value, len) };
    }
}

// SAFETY: Delegates to `__memset_fallible`,
// which handles page faults when filling fallible memory.
unsafe impl Memset for Fallible {
    type Result = usize;

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) -> usize {
        // SAFETY: The safety is upheld by the caller.
        let failed_bytes = unsafe { __memset_fallible(dst, value, len) };
        len - failed_bytes
    }
}
