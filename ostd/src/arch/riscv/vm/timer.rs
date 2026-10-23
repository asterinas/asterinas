// SPDX-License-Identifier: MPL-2.0

use super::csr::Vstimecmp;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(super) struct VcpuTimer {
    /// Next timer event cycles.
    next_cycles: usize,
}

impl VcpuTimer {
    pub(super) fn init(&mut self) {
        self.next_cycles = 0;
    }

    pub(super) fn sync(&mut self) {
        self.next_cycles = Vstimecmp::read();
    }

    pub(super) fn reset(&mut self) {
        self.next_cycles = usize::MAX;
    }

    pub(super) fn restore(&mut self) {
        Vstimecmp::write(self.next_cycles);
    }
}
