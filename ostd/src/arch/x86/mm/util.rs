// SPDX-License-Identifier: MPL-2.0

core::arch::global_asm!(include_str!("memcpy_fallible.S"));
core::arch::global_asm!(include_str!("memset_fallible.S"));

core::arch::global_asm!(include_str!("atomic_load_fallible.S"));
core::arch::global_asm!(include_str!("atomic_cmpxchg_fallible.S"));

extern "C" {
    /// Copies `size` bytes from `src` to `dst`. This function works with exception handling
    /// and can recover from page fault.
    /// Returns number of bytes that failed to copy.
    pub(crate) fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize;
    /// Fills `size` bytes in the memory pointed to by `dst` with the value `value`.
    /// This function works with exception handling and can recover from page fault.
    /// Returns number of bytes that failed to set.
    pub(crate) fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize;

    /// Atomically loads a 32-bit integer value. This function works with exception handling
    /// and can recover from page fault.
    /// Returns the loaded value or `!0u64` if failed to load.
    pub(crate) fn __atomic_load_fallible(ptr: *const u32) -> u64;
    /// Atomically compares and exchanges a 32-bit integer value. This function works with
    /// exception handling and can recover from page fault.
    /// Returns the previous value or `!0u64` if failed to update.
    pub(crate) fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64;
}
