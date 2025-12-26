// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{mm::page_size_at, task::disable_preempt};

use super::{PteRangeMeta, RsAsDelta, Vmar, VmarCursorMut};
use crate::{
    prelude::*,
    vm::vmar::{
        VMAR_CAP_ADDR,
        cursor_util::{take_next_unmappable, unmap_count_rs_as},
        interval_set::Interval,
        vm_allocator::PerCpuAllocatorGuard,
    },
};

impl Vmar {
    /// Clears all mappings.
    ///
    /// After being cleared, this vmar will become an empty vmar
    #[expect(dead_code)] // TODO: This should be called when the last process drops the VMAR.
    pub fn clear(&self) {
        let preempt_guard = disable_preempt();
        let full_range = 0..VMAR_CAP_ADDR;
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &full_range)
            .unwrap();

        debug_assert_eq!(cursor.level(), cursor.guard_level());
        cursor.aux_meta_mut().inner.clear();

        while cursor
            .find_next_unmappable_subtree(full_range.end - cursor.virt_addr())
            .is_some()
        {
            cursor.unmap();
        }

        self.allocator.reset();

        self.rss_counters
            .iter()
            .for_each(|counter| counter.reset_all_zero());

        self.total_vm.reset_all_zero();

        cursor.flusher().dispatch_tlb_flush();
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
        let mut rs_as_delta = RsAsDelta::new(self);

        let preempt_guard = disable_preempt();
        let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range).unwrap();
        let mut dealloc_guard = self.allocator.lock_for_dealloc();

        self.remove_mappings(
            &mut cursor,
            range.len(),
            &mut rs_as_delta,
            Some(&mut dealloc_guard),
        )?;

        Ok(())
    }

    /// Removes mappings in the given range using the provided cursor.
    ///
    /// if `dealloc` is true, it also deallocates the removed mappings' ranges
    /// to the allocator.
    pub(super) fn remove_mappings(
        &self,
        cursor: &mut VmarCursorMut<'_>,
        len: usize,
        rs_as_delta: &mut RsAsDelta,
        mut dealloc_to: Option<&mut PerCpuAllocatorGuard<'_>>,
    ) -> Result<()> {
        let range = cursor.virt_addr()..cursor.virt_addr() + len;

        while let Some(meta) = take_next_unmappable(cursor, range.end) {
            match meta {
                PteRangeMeta::ChildPt(range) => {
                    let cur_page_size = page_size_at(cursor.level());
                    for child_va in range.step_by(cur_page_size) {
                        cursor.jump(child_va).unwrap();

                        // Temporarily insert the `ChildPt` meta back to allow
                        // `unmap_count_rs_as` correctly navigating the child page table.
                        cursor
                            .aux_meta_mut()
                            .inner
                            .insert(PteRangeMeta::ChildPt(child_va..child_va + cur_page_size));
                        let num_unmapped = unmap_count_rs_as(
                            cursor,
                            child_va + cur_page_size,
                            rs_as_delta,
                            &mut dealloc_to,
                        );
                        cursor.aux_meta_mut().inner.take_one(&child_va).unwrap();

                        let num_cursor_unmapped = cursor.unmap();

                        debug_assert_eq!(num_unmapped, num_cursor_unmapped);
                    }
                }
                PteRangeMeta::VmMapping(vm_mapping) => {
                    let taken_range = vm_mapping.range();
                    rs_as_delta.add_as(-(taken_range.len() as isize));

                    if let Some(allocator) = dealloc_to.as_mut() {
                        allocator.dealloc(taken_range.clone());
                    }
                    vm_mapping.unmap(cursor, rs_as_delta);

                    if cursor.jump(taken_range.end).is_err() {
                        break;
                    };
                }
            }
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Ok(())
    }
}
