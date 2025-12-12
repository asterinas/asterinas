// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::mm::Vaddr;

/// A virtual range and its relocated address.
pub(super) struct RelocatedRange {
    original_range: Range<Vaddr>,
    relocated_start: Vaddr,
}

impl RelocatedRange {
    /// Creates a new `RelocatedRange`.
    ///
    /// If the relocated address overflows, it will return `None`.
    pub(super) fn new(original_range: Range<Vaddr>, relocated_start: Vaddr) -> Option<Self> {
        relocated_start.checked_add(original_range.len())?;
        Some(Self {
            original_range,
            relocated_start,
        })
    }

    /// Gets the relocated address of an address in the original range.
    ///
    /// If the provided address is not in the original range, it will return `None`.
    pub(super) fn relocated_addr_of(&self, addr: Vaddr) -> Option<Vaddr> {
        if self.original_range.contains(&addr) {
            Some(addr - self.original_range.start + self.relocated_start)
        } else {
            None
        }
    }

    /// Returns the relocated start address.
    pub(super) fn relocated_start(&self) -> Vaddr {
        self.relocated_start
    }
}
