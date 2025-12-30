// SPDX-License-Identifier: MPL-2.0

use core::{cmp, ops::Range};

use align_ext::AlignExt;
use ostd::sync::RwMutexReadGuard;

use super::{VmMapping, Vmar, VmarInner};
use crate::{prelude::*, vm::shared_mem::AttachedShm};

impl Vmar {
    /// Finds all the mapped regions that intersect with the specified range.
    pub fn query(&self, range: Range<usize>) -> VmarQueryGuard<'_> {
        VmarQueryGuard {
            vmar: self.inner.read(),
            range,
        }
    }

    /// Finds the first SysV shared memory attachment whose mapped range
    /// intersects within the given range.
    pub fn find_first_shm_in_range(
        &self,
        addr: Vaddr,
        length: usize,
    ) -> Result<(AttachedShm, usize)> {
        let scan_end = addr
            .checked_add(length)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "overflow in shm scan range"))?;

        for mapping in self.query(addr..scan_end).iter() {
            let Some(attached_shm) = mapping.attached_shm() else {
                continue;
            };
            let Some(vmo_offset) = mapping.vmo_offset() else {
                continue;
            };
            let Some(vmo_size) = mapping.vmo_size() else {
                continue;
            };

            // The mappings matches if its virtual start address, minus its
            // page offset in the shared-memory object, equals the given addr.
            if mapping.map_to_addr().checked_sub(vmo_offset) != Some(addr) {
                continue;
            }

            return Ok((attached_shm, vmo_size.align_up(PAGE_SIZE)));
        }

        return_errno!(Errno::EINVAL);
    }

    /// Collects the mapped ranges (merged and page-aligned) that belong to the
    /// specified shared memory attachment and intersect with `range`.
    pub fn collect_attached_shm_ranges(
        &self,
        range: Range<Vaddr>,
        attached_shm: AttachedShm,
    ) -> Vec<Range<Vaddr>> {
        let mut ranges: Vec<Range<Vaddr>> = Vec::new();
        for mapping in self.query(range.clone()).iter() {
            if mapping.attached_shm() != Some(attached_shm) {
                continue;
            }
            let start = cmp::max(mapping.map_to_addr(), range.start);
            let end = cmp::min(mapping.map_end(), range.end);
            if start < end {
                ranges.push(start..end);
            }
        }

        ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<Vaddr>> = Vec::new();
        for r in ranges {
            if let Some(last) = merged.last_mut()
                && last.end == r.start
            {
                last.end = r.end;
            } else {
                merged.push(r);
            }
        }

        merged
    }
}

/// A guard that allows querying a [`Vmar`] for its mappings.
pub struct VmarQueryGuard<'a> {
    vmar: RwMutexReadGuard<'a, VmarInner>,
    range: Range<usize>,
}

impl VmarQueryGuard<'_> {
    /// Returns an iterator over the [`VmMapping`]s that intersect with the
    /// provided range when calling [`Vmar::query`].
    pub fn iter(&self) -> impl Iterator<Item = &VmMapping> {
        self.vmar.query(&self.range)
    }

    /// Returns whether the range is fully mapped.
    ///
    /// In other words, this method will return `false` if and only if the
    /// range contains pages that are not mapped.
    pub fn is_fully_mapped(&self) -> bool {
        let mut last_mapping_end = self.range.start;

        for mapping in self.iter() {
            if last_mapping_end < mapping.map_to_addr() {
                return false;
            }
            last_mapping_end = mapping.map_end();
        }

        if last_mapping_end < self.range.end {
            return false;
        }

        true
    }
}
