// SPDX-License-Identifier: MPL-2.0

use crate::arch::vm::csr::Vstimecmp;

/// Reads the value of the `Vstimecmp` register as needed.
pub fn read_timer_cmp() -> usize {
    // SAFETY: The guest has already exited at the time of the call; furthermore,
    // `Vstimecmp` functions solely as a counter and has no impact on memory safety.
    unsafe { Vstimecmp::read() }
}

/// Writes a value to the `Vstimecmp` register as needed.
pub fn write_timer_cmp(cmp: usize) {
    // SAFETY: At the time of the call, control is about to return to the guest; furthermore,
    // `Vstimecmp` functions solely as a counter and has no impact on memory safety.
    unsafe {
        Vstimecmp::write(cmp);
    }
}
