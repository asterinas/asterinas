// SPDX-License-Identifier: MPL-2.0

use core::{array, num::NonZeroUsize, ops::Range};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::CpuId,
    mm::{
        CachePolicy, HasSize, MAX_USERSPACE_VADDR, PageFlags, UFrame, VmSpace,
        io_util::HasVmReaderWriter,
        page_size, page_size_at,
        tlb::TlbFlushOp,
        vm_space::{Cursor, CursorMut, VmQueriedItem},
    },
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
};

use super::{
    Interval, IntervalSet, MappedMemory, MappedVmo, PerCpuAllocator, RssDelta, RssType, VmMapping,
    Vmar, find_next_mapped, find_next_unmappable, propagate_if_needed, split_and_insert_rest,
};
use crate::{
    fs::{file_handle::Mappable, ramfs::memfd::MemfdInode},
    prelude::*,
    process::{Process, ProcessVm, ResourceType, posix_thread::last_tid},
    thread::exception::PageFaultInfo,
    vm::{self, perms::VmPerms, vmar::cursor_utils::unmap_count_rss, vmo::Vmo},
};

impl Vmar {
    /// Clears all mappings.
    ///
    /// After being cleared, this vmar will become an empty vmar
    #[expect(dead_code)] // TODO: This should be called when the last process drops the VMAR.
    pub fn clear(&self) {
        let preempt_guard = disable_preempt();
        let full_range = 0..MAX_USERSPACE_VADDR;
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &full_range)
            .unwrap();

        debug_assert_eq!(cursor.level(), cursor.guard_level());
        cursor.aux_meta().inner.clear();

        while cursor
            .find_next_unmappable_subtree(full_range.end - cursor.virt_addr())
            .is_some()
        {
            cursor.unmap();
        }

        self.rss_counters
            .iter()
            .for_each(|counter| counter.reset_all_zero());

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
        let mut rss_delta = RssDelta::new(self);

        let preempt_guard = disable_preempt();
        let range = self.range();
        let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        while let Some(va_range) = find_next_unmappable(&mut cursor, range.end - cursor.virt_addr())
        {
            let intersected_range = get_intersected_range(&range, &va_range);
            cursor.jump(intersected_range.start).unwrap();
            cursor.propagate_if_needed(intersected_range.len());

            let Some(meta) = cursor.aux_meta().inner.remove(va) else {
                panic!("`find_next_unmappable` does not stop at unmappable subtree");
            };

            match meta {
                PteRangeMeta::ChildPt(range) => {
                    for child_va in range.step_by(page_size_at(cursor.level())) {
                        cursor.jump(child_va).unwrap();
                        unmap_count_rss(&mut cursor, len, &mut rss_delta);
                        let _ = cursor.unmap();
                    }
                }
                PteRangeMeta::VmMapping(vm_mapping) => {
                    let taken =
                        split_and_insert_rest(&mut cursor, vm_mapping, intersected_range.clone());

                    let next_address = taken.range().end;
                    taken.unmap(&mut cursor, &mut rss_delta);
                    cursor.jump(next_address).unwrap();
                }
            }
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Ok(())
    }
}
