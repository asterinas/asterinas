// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use super::{Interval, RssDelta, VMAR_CAP_ADDR, Vmar, util::get_intersected_range};
use crate::{prelude::*, process::FutureMemoryLock};

impl Vmar {
    /// Locks the memory mappings in the specified range.
    ///
    /// The range's start and end addresses must be page-aligned.
    ///
    /// If the range contains unmapped pages, an [`ENOMEM`] error will be returned.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub fn lock_range(&self, range: Range<Vaddr>, limit: Option<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();
        let new_locked_size = inner.new_locked_size(&range)?;

        if let Some(limit) = limit
            && inner.locked_vm.saturating_add(new_locked_size) > limit
        {
            return_errno_with_message!(Errno::ENOMEM, "the memory lock limit is reached");
        }

        let mut rss_delta = RssDelta::new(self);
        inner.populate_range(&self.vm_space, &range, &mut rss_delta)?;
        inner.set_mapping_locks(range, true);
        inner.locked_vm += new_locked_size;

        Ok(())
    }

    /// Applies `mlockall` to current and/or future memory mappings.
    pub fn lock_all(
        &self,
        lock_current: bool,
        future_memory_lock: FutureMemoryLock,
        limit: Option<usize>,
    ) -> Result<()> {
        let mut inner = self.inner.write();

        if lock_current {
            let new_locked_size = inner.all_new_locked_size();

            if let Some(limit) = limit
                && inner.locked_vm.saturating_add(new_locked_size) > limit
            {
                return_errno_with_message!(Errno::ENOMEM, "the memory lock limit is reached");
            }

            let mut rss_delta = RssDelta::new(self);
            inner.populate_range(&self.vm_space, &(0..VMAR_CAP_ADDR), &mut rss_delta)?;
            inner.set_mapping_locks(0..VMAR_CAP_ADDR, true);
            inner.locked_vm += new_locked_size;
        }

        inner.future_memory_lock = future_memory_lock;

        Ok(())
    }

    /// Unlocks the memory mappings in the specified range.
    ///
    /// The range's start and end addresses must be page-aligned.
    pub fn unlock_range(&self, range: Range<Vaddr>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();
        inner.check_range_mapped(&range)?;
        let locked_size = inner.locked_size(&range);
        inner.set_mapping_locks(range, false);
        inner.locked_vm = inner.locked_vm.saturating_sub(locked_size);

        Ok(())
    }

    /// Unlocks all memory mappings.
    pub fn unlock_all(&self) {
        let mut inner = self.inner.write();
        let full_range = 0..VMAR_CAP_ADDR;
        inner.set_mapping_locks(full_range, false);
        inner.locked_vm = 0;
        inner.future_memory_lock = FutureMemoryLock::Disabled;
    }
}

impl super::VmarInner {
    fn new_locked_size(&self, range: &Range<Vaddr>) -> Result<usize> {
        let mut new_locked_size = 0;
        let mut last_mapping_end = range.start;

        for vm_mapping in self.vm_mappings.find(range) {
            if last_mapping_end < vm_mapping.map_to_addr() {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "the range contains pages that are not mapped"
                );
            }
            last_mapping_end = vm_mapping.map_end();

            if vm_mapping.is_locked() {
                continue;
            }

            let intersected_range = get_intersected_range(range, &vm_mapping.range());
            new_locked_size += intersected_range.len();
        }

        if last_mapping_end < range.end {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            );
        }

        Ok(new_locked_size)
    }

    fn check_range_mapped(&self, range: &Range<Vaddr>) -> Result<()> {
        let mut last_mapping_end = range.start;

        for vm_mapping in self.vm_mappings.find(range) {
            if last_mapping_end < vm_mapping.map_to_addr() {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "the range contains pages that are not mapped"
                );
            }
            last_mapping_end = vm_mapping.map_end();
        }

        if last_mapping_end < range.end {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            );
        }

        Ok(())
    }

    pub(super) fn check_extra_lock_size_fits_limit(
        &self,
        extra_lock_size: usize,
        limit: Option<usize>,
    ) -> Result<()> {
        if let Some(limit) = limit
            && self.locked_vm.saturating_add(extra_lock_size) > limit
        {
            return_errno_with_message!(Errno::ENOMEM, "the memory lock limit is reached");
        }

        Ok(())
    }

    fn all_new_locked_size(&self) -> usize {
        let mut new_locked_size = 0;

        for vm_mapping in self.vm_mappings.iter() {
            if !vm_mapping.is_locked() {
                new_locked_size += vm_mapping.map_size();
            }
        }

        new_locked_size
    }

    fn populate_range(
        &self,
        vm_space: &ostd::mm::VmSpace,
        range: &Range<Vaddr>,
        rss_delta: &mut RssDelta,
    ) -> Result<()> {
        for vm_mapping in self.vm_mappings.find(range) {
            let intersected_range = get_intersected_range(range, &vm_mapping.range());
            vm_mapping.populate_range(vm_space, &intersected_range, rss_delta)?;
        }

        Ok(())
    }

    pub(super) fn locked_size(&self, range: &Range<Vaddr>) -> usize {
        let mut locked_size = 0;

        for vm_mapping in self.vm_mappings.find(range) {
            if !vm_mapping.is_locked() {
                continue;
            }

            let intersected_range = get_intersected_range(range, &vm_mapping.range());
            locked_size += intersected_range.len();
        }

        locked_size
    }

    fn set_mapping_locks(&mut self, range: Range<Vaddr>, is_locked: bool) {
        let mut mappings_to_update = Vec::new();

        for vm_mapping in self.vm_mappings.find(&range) {
            mappings_to_update.push(vm_mapping.range());
        }

        for vm_mapping_range in mappings_to_update {
            let Some(vm_mapping) = self.remove(&vm_mapping_range.start) else {
                continue;
            };

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            if let Some(left) = left {
                self.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                self.insert_without_try_merge(right);
            }

            self.insert_try_merge(taken.set_locked(is_locked));
        }
    }
}
