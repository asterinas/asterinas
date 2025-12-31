// SPDX-License-Identifier: MPL-2.0

use core::{ops::Range, sync::atomic::Ordering};

use ostd::{cpu::all_cpus, mm::page_size_at, task::disable_preempt};

use super::{PteRangeMeta, RsAsDelta, Vmar, VmarCursorMut};
use crate::{
    prelude::*,
    vm::vmar::{
        VMAR_CAP_ADDR,
        cursor_util::{find_next_mapped, take_next_unmappable, unmap_count_rs_as},
        interval_set::Interval,
        util::get_intersected_range,
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

        self.rss_counters
            .iter()
            .for_each(|counter| counter.reset_all_zero());

        all_cpus().for_each(|cpu| {
            self.mapped_vm_size
                .get_on_cpu(cpu)
                .store(0, Ordering::Release)
        });

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

        self.remove_mappings(&mut cursor, range.len(), &mut rs_as_delta)?;

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
                        let num_unmapped =
                            unmap_count_rs_as(cursor, child_va + cur_page_size, rs_as_delta);
                        cursor.aux_meta_mut().inner.take_one(&child_va).unwrap();

                        let num_cursor_unmapped = cursor.unmap();

                        debug_assert_eq!(num_unmapped, num_cursor_unmapped);
                    }
                }
                PteRangeMeta::VmMapping(vm_mapping) => {
                    let taken_range = vm_mapping.range();

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

        let mut rs_as_delta = RsAsDelta::new(self);

        let mut last_mapping_end = range.start;

        let preempt_guard = disable_preempt();
        let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        while let Some(vm_mapping) = find_next_mapped!(cursor, range.end) {
            let vm_mapping_level = cursor.level();
            let vm_mapping_range = vm_mapping.range();
            let rss_type = vm_mapping.rss_type();

            // The range may contain pages that are not mapped. According to Linux behavior, this
            // is not a fault. However, an `ENOMEM` should be reported at the end.
            if last_mapping_end >= vm_mapping.map_to_addr() {
                last_mapping_end = vm_mapping.map_end();
            }

            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            cursor.jump(intersected_range.start).unwrap();
            let page_size = page_size_at(vm_mapping_level);

            let mut num_unmapped_this_mapping = 0;

            while let Some(va) = cursor.find_next(intersected_range.end - cursor.virt_addr()) {
                let num_unmapped = cursor.unmap();
                rs_as_delta.add_rs(rss_type, -(num_unmapped as isize));
                num_unmapped_this_mapping += num_unmapped;

                if va + page_size < intersected_range.end {
                    cursor.jump(va + page_size).unwrap();
                } else {
                    break;
                }
            }

            let Some(PteRangeMeta::VmMapping(mapping)) = cursor
                .aux_meta_mut()
                .inner
                .find_one_mut(&intersected_range.start)
            else {
                panic!("there were mappings but cannot find it the second time");
            };
            mapping.dec_frames_mapped(num_unmapped_this_mapping);

            if cursor.jump(intersected_range.end).is_err() {
                break;
            }
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
