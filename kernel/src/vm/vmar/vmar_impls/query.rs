// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::sync::RwMutexReadGuard;

use super::{VmMapping, Vmar, VmarInner};

impl Vmar {
    /// Finds all the mapped regions that intersect with the specified range.
    pub fn query(&self, range: Range<usize>) -> VmarQueryGuard<'_> {
        VmarQueryGuard {
            vmar: self.inner.read(),
            range,
        }
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
