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

        let mut protect_mappings = Vec::new();

        for vm_mapping in inner.vm_mappings.find(&range) {
            protect_mappings.push((vm_mapping.range(), vm_mapping.perms()))
        }

        let mut last_mapping_end = range.start;
        for (vm_mapping_range, vm_mapping_perms) in protect_mappings {
            if last_mapping_end < vm_mapping_range.start {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "the range contains pages that are not mapped"
                );
            }
            last_mapping_end = vm_mapping_range.end;

            if perms == vm_mapping_perms & VmPerms::ALL_PERMS {
                continue;
            }
            let new_perms = perms | (vm_mapping_perms & VmPerms::ALL_MAY_PERMS);
            new_perms.check()?;

            let vm_mapping = inner.remove(&vm_mapping_range.start).unwrap();
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            // Protects part of the taken `VmMapping`.
            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            // Puts the rest back.
            if let Some(left) = left {
                inner.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(right);
            }

            // Protects part of the `VmMapping`.
            let taken = taken.protect(vm_space.as_ref(), new_perms);
            inner.insert_try_merge(taken);
        }

        if last_mapping_end < range.end {
            return_errno_with_message!(
                Errno::ENOMEM,
                "the range contains pages that are not mapped"
            );
        }

        Ok(())
    }
}
