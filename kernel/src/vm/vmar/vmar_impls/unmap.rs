// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::task::disable_preempt;

use super::{RssDelta, Vmar};
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

impl Vmar {
    /// Clears all mappings.
    ///
    /// After being cleared, this vmar will become an empty vmar
    #[expect(dead_code)] // TODO: This should be called when the last process drops the VMAR.
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.vm_mappings.clear();

        // Keep `inner` locked to avoid race conditions.
        let preempt_guard = disable_preempt();
        let full_range = 0..VMAR_CAP_ADDR;
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &full_range)
            .unwrap();
        cursor.unmap(full_range.len());
        cursor.flusher().sync_tlb_flush();
    }

    /// Destroys all mappings that fall within the specified
    /// range in bytes.
    ///
    /// The range's start and end addresses must be page-aligned.
    ///
    /// Mappings may fall partially within the range; only the overlapped
    /// portions of the mappings are unmapped.
    pub fn remove_mapping(&self, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);
        inner.alloc_free_region_exact_truncate(
            &self.vm_space,
            range.start,
            range.len(),
            &mut rss_delta,
        )?;
        Ok(())
    }
}
