// SPDX-License-Identifier: MPL-2.0

//! Kernel-side helpers for VM-space cursor operations.

use core::ops::Range;

use ostd::mm::{Vaddr, vm_space::CursorMut};

pub(super) trait CursorMutExt {
    /// Splits the current mapping until its virtual address range is contained in `range`.
    fn split_if_map_exceeds_range(&mut self, range: &Range<Vaddr>);
}

impl CursorMutExt for CursorMut<'_> {
    fn split_if_map_exceeds_range(&mut self, range: &Range<Vaddr>) {
        while self.cur_va_range().start < range.start || self.cur_va_range().end > range.end {
            self.adjust_level(self.level() - 1);
        }
    }
}
