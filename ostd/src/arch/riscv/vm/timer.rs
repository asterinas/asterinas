// SPDX-License-Identifier: MPL-2.0

use crate::arch::vm::csr::Vstimecmp;

/// The Timer of Vcpu.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct VcpuTimer {
    /// Next timer event cycles.
    next_cycles: usize,
}

impl VcpuTimer {
    /// Init the timer.
    pub fn init(&mut self) {
        self.next_cycles = 0;
    }

    pub(super) fn sync(&mut self) {
        // SAFETY: The guest has already exited at the time of the call; furthermore,
        // `Vstimecmp` functions solely as a counter and has no impact on memory safety.
        unsafe {
            self.next_cycles = Vstimecmp::read();
        }
    }

    /// Reset the timer.
    pub fn reset(&mut self) {
        self.next_cycles = usize::MAX;
    }

    pub(super) fn save(&self) {
        // SAFETY: When returning to user mode, registers must be reset; furthermore,
        // `Vstimecmp` functions solely as a counter and has no impact on memory safety.
        unsafe {
            Vstimecmp::write(usize::MAX);
        }
    }

    pub(super) fn restore(&mut self) {
        // SAFETY: At the time of the call, control is about to return to the guest; furthermore,
        // `Vstimecmp` functions solely as a counter and has no impact on memory safety.
        unsafe {
            Vstimecmp::write(self.next_cycles);
        }
    }
}
