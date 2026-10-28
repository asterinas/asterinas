// SPDX-License-Identifier: MPL-2.0

use super::{node::Extent, path::ExtentMappingRange};
use crate::fs::fs_impls::ext4::prelude::*;

/// Bounded, reconstructible lookup hints invalidated together after mapping edits.
#[derive(Debug, Default)]
pub(super) struct LookupCache {
    status: ExtentStatusTree,
    last: LastExtent,
}

impl LookupCache {
    pub(super) fn lookup(&self, iblock: Iblock, end: Iblock) -> Option<ExtentMappingRange> {
        if let Some(extent) = self.last.lookup(iblock) {
            return Some(ExtentMappingRange::Mapped {
                logical: iblock..end.min(extent.logical_end() as Iblock),
                extent,
            });
        }
        self.status
            .lookup(iblock)
            .map(|mapping_range| mapping_range.clip(iblock, end))
    }

    pub(super) fn record(&mut self, mapping_range: ExtentMappingRange) {
        if let ExtentMappingRange::Mapped { extent, .. } = &mapping_range {
            self.last.record(*extent);
        }
        self.status.record(mapping_range);
    }

    pub(super) fn covers_initialized(&self, range: Range<Iblock>) -> bool {
        self.status.covers_initialized(range)
    }

    pub(super) fn invalidate(&mut self, range: Range<Iblock>) {
        self.status.invalidate(range.clone());
        self.last.invalidate(range);
    }
}

/// Reconstructible logical mapping facts. A missing entry means unknown,
/// never a hole. Overlapping edits discard the entire old fact conservatively.
#[derive(Debug, Default)]
struct ExtentStatusTree {
    entries: BTreeMap<Iblock, ExtentMappingRange>,
}

/// One validated extent, invalidated with the logical status index on edits.
#[derive(Debug, Default)]
struct LastExtent(Option<Extent>);

impl ExtentStatusTree {
    fn lookup(&self, iblock: Iblock) -> Option<ExtentMappingRange> {
        let start = iblock;
        self.entries
            .range(..=start)
            .next_back()
            .filter(|(_, mapping_range)| mapping_range.bounds().contains(&start))
            .map(|(_, mapping_range)| mapping_range.clone())
    }

    fn record(&mut self, mapping_range: ExtentMappingRange) {
        if mapping_range.bounds().is_empty() {
            return;
        }
        self.invalidate(mapping_range.bounds().clone());
        if self.entries.len() >= MAX_STATUS_ENTRIES {
            self.entries.pop_first();
        }
        self.entries
            .insert(mapping_range.bounds().start, mapping_range);
    }

    fn invalidate(&mut self, range: Range<Iblock>) {
        let mut keys: Vec<_> = self
            .entries
            .range(range.clone())
            .map(|(&key, _)| key)
            .collect();
        if let Some((&key, mapping_range)) = self.entries.range(..range.start).next_back()
            && mapping_range.bounds().end > range.start
        {
            keys.push(key);
        }
        for key in keys {
            self.entries.remove(&key);
        }
    }

    fn covers_initialized(&self, range: Range<Iblock>) -> bool {
        let mut start = range.start;
        while start < range.end {
            let Some(ExtentMappingRange::Mapped { logical, extent }) = self.lookup(start) else {
                return false;
            };
            if extent.is_unwritten() {
                return false;
            }
            start = logical.end;
        }
        true
    }
}

impl LastExtent {
    fn lookup(&self, iblock: Iblock) -> Option<Extent> {
        self.0.filter(|e| e.covers(iblock))
    }
    fn record(&mut self, extent: Extent) {
        self.0 = Some(extent);
    }
    fn invalidate(&mut self, range: Range<Iblock>) {
        if self
            .0
            .is_some_and(|e| e.block() < range.end && e.logical_end() > u64::from(range.start))
        {
            self.0 = None;
        }
    }
}

const MAX_STATUS_ENTRIES: usize = 512;

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn status_tree_tracks_coverage_invalidation_and_eviction() {
        let mut status = ExtentStatusTree::default();
        assert!(status.lookup(7).is_none());
        status.record(ExtentMappingRange::Hole(0..10));
        status.invalidate(5..6);
        assert!(status.lookup(4).is_none());
        for i in 0..513u32 {
            status.record(ExtentMappingRange::Hole(i * 2..i * 2 + 1));
        }
        assert!(status.lookup(0).is_none());
        assert!(status.lookup(2).is_some());
        assert!(matches!(
            status.lookup(1024),
            Some(ExtentMappingRange::Hole(_))
        ));
        status.record(ExtentMappingRange::Mapped {
            logical: 2000..2001,
            extent: Extent::new(2000, 1, 50),
        });
        status.record(ExtentMappingRange::Mapped {
            logical: 2001..2002,
            extent: Extent::new(2001, 1, 51),
        });
        assert!(status.covers_initialized(2000..2002));
        assert!(!status.covers_initialized(1999..2002));
    }
}
