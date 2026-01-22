// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use super::{Interval, Vmar, util::get_intersected_range};
use crate::{prelude::*, vm::perms::VmPerms};

impl Vmar {
    /// Change the permissions of the memory mappings in the specified range.
    ///
    /// The range's start and end addresses must be page-aligned.
    ///
    /// If the range contains unmapped pages, an [`ENOMEM`] error will be returned.
    /// Note that pages before the unmapped hole are still protected.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();
        let vm_space = self.vm_space();

        // Step 1: Validate mappings and collect their start addresses
        let mut starts = Vec::new();
        let mut last_end = range.start;
        let mut actual_end = range.start;

        for vm_mapping in inner.vm_mappings.find(&range) {
            let r = vm_mapping.range();

            // Check for gaps between mappings
            if last_end < r.start {
                actual_end = last_end;
                break;
            }

            starts.push(r.start);
            last_end = r.end;
        }

        if actual_end == range.start {
            actual_end = last_end;
        }

        // Determine if the full range is covered
        let full_range_covered = actual_end >= range.end;
        let protect_end = if full_range_covered {
            range.end
        } else {
            actual_end
        };

        if protect_end == range.start {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            );
        }

        // Step 2: Remove all affected mappings from the tree
        let mut mappings = Vec::new();
        for start in starts {
            if let Some(m) = inner.remove(&start) {
                mappings.push(m);
            }
        }

        // Step 3: Process each mapping - split, protect, and reinsert
        let protect_range = range.start..protect_end;

        for vm_mapping in mappings {
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&protect_range, &vm_mapping_range);

            if intersected_range.start == intersected_range.end {
                inner.insert_try_merge(vm_mapping);
                continue;
            }

            // Protects part of the taken `VmMapping`.
            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            // Puts the rest back.
            if let Some(left) = left {
                inner.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(right);
            }

            let old_perms = taken.perms();
            if perms == (old_perms & VmPerms::ALL_PERMS) {
                inner.insert_try_merge(taken);
                continue;
            }

            let new_perms = perms | (old_perms & VmPerms::ALL_MAY_PERMS);
            new_perms.check()?;

            // Protects part of the `VmMapping`.
            let taken = taken.protect(vm_space.as_ref(), new_perms);
            inner.insert_try_merge(taken);
        }

        if !full_range_covered {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            );
        }

        Ok(())
    }
}
