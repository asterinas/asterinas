// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::task::disable_preempt;

use super::{RssDelta, Vmar};
use crate::{
    prelude::*,
    vm::vmar::{VMAR_CAP_ADDR, interval_set::Interval, util::get_intersected_range},
};

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

    /// Discards all pages in the mappings that fall within the specified
    /// range in bytes.
    ///
    /// The pages will be reloaded from a clean state by the page fault
    /// handler if they are accessed later.
    ///
    /// The range's start and end addresses must be page-aligned.
    ///
    /// Mappings may fall partially within the range; only the pages in the
    /// overlapped portions of the mappings are discarded.
    ///
    /// If the range contains unmapped pages, an [`ENOMEM`] error will be returned.
    /// Note that all other pages are still discarded.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub fn discard_pages(&self, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let inner = self.inner.read();
        let mut rss_delta = RssDelta::new(self);

        let mut last_mapping_end = range.start;
        for vm_mapping in inner.vm_mappings.find(&range) {
            if !vm_mapping.can_expand() {
                return_errno_with_message!(Errno::EINVAL, "device mappings cannot be discarded");
            }

            // The range may contain pages that are not mapped. According to Linux behavior, this
            // is not a fault. However, an `ENOMEM` should be reported at the end.
            if last_mapping_end >= vm_mapping.map_to_addr() {
                last_mapping_end = vm_mapping.map_end();
            }

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let preempt_guard = disable_preempt();
            let vm_space = self.vm_space();
            let mut cursor = vm_space
                .cursor_mut(&preempt_guard, &intersected_range)
                .unwrap();

            rss_delta.add(
                vm_mapping.rss_type(),
                -(cursor.unmap(intersected_range.len()) as isize),
            );
            cursor.flusher().dispatch_tlb_flush();
            cursor.flusher().sync_tlb_flush();
        }

        if last_mapping_end < range.end {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            )
        }

        Ok(())
    }
}
