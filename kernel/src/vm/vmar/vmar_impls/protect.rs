// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{
    mm::{PAGE_SIZE, Vaddr},
    task::disable_preempt,
};

use super::{PteRangeMeta, Vmar};
use crate::{
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{
            cursor_util::{find_next_mapped, propagate_if_needed, split_and_insert_rest},
            interval_set::Interval,
            util::get_intersected_range,
        },
    },
};

impl Vmar {
    /// Change the permissions of the memory mappings in the specified range.
    ///
    /// # Errors
    ///
    /// Returns [`Errno::ENOMEM`] if the range covers pages that are not mapped.
    /// Note that on returning, virtual addresses before the unmapped page are
    /// already protected. This is compatible with Linux 6.17.
    ///
    /// # Panics
    ///
    /// Panics if the range's start and end addresses are not page-aligned.
    pub fn protect(&self, perms: VmPerms, range: Range<Vaddr>) -> Result<()> {
        assert!(range.start.is_multiple_of(PAGE_SIZE));
        assert!(range.end.is_multiple_of(PAGE_SIZE));

        // To check if any pages are not mapped.
        let mut last_protected_addr = range.start;

        let preempt_guard = disable_preempt();
        let vm_space = self.vm_space();
        let mut cursor = vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        while let Some(vm_mapping) = find_next_mapped!(cursor, range.end) {
            let vm_mapping_range = vm_mapping.range();

            if last_protected_addr < vm_mapping_range.start {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "protect: the range covers unmapped pages"
                );
            }

            // Skip if no actions needed.
            if perms == vm_mapping.perms() & VmPerms::ALL_PERMS {
                if vm_mapping_range.end >= range.end {
                    last_protected_addr = range.end;
                    break;
                } else {
                    cursor.jump(vm_mapping_range.end).unwrap();
                    last_protected_addr = vm_mapping_range.end;
                    continue;
                }
            }

            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            cursor.jump(intersected_range.start).unwrap();
            propagate_if_needed(&mut cursor, intersected_range.len());

            let Some(PteRangeMeta::VmMapping(vm_mapping)) = cursor
                .aux_meta_mut()
                .inner
                .take_one(&intersected_range.start)
            else {
                panic!("`find_next_mapped` does not stop at mapped `VmMapping`");
            };

            let new_perms = perms | (vm_mapping.perms() & VmPerms::ALL_MAY_PERMS);
            new_perms.check()?;

            let taken = split_and_insert_rest(&mut cursor, vm_mapping, intersected_range.clone());
            let next_address = taken.range().end;

            let taken = taken.protect(&mut cursor, new_perms);
            cursor.aux_meta_mut().insert_try_merge(taken);

            last_protected_addr = next_address;

            if cursor.jump(next_address).is_err() {
                break;
            }
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        if last_protected_addr < range.end {
            return_errno_with_message!(Errno::ENOMEM, "protect: the range covers unmapped pages");
        }

        Ok(())
    }
}
