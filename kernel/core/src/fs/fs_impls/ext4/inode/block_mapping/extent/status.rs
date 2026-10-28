// SPDX-License-Identifier: MPL-2.0

use super::{node::Extent, path::ExtentRun};
use crate::fs::fs_impls::ext4::prelude::*;

/// Bounded, reconstructible lookup hints invalidated together after mapping edits.
#[derive(Debug, Default)]
pub(super) struct LookupCache {
    status: ExtentStatusTree,
    last: LastExtent,
}

impl LookupCache {
    pub(super) fn lookup(&self, iblock: Iblock, end: u64) -> Option<ExtentRun> {
        if let Some(extent) = self.last.lookup(iblock) {
            return Some(ExtentRun::Mapped {
                logical: u64::from(iblock)..end.min(extent.logical_end()),
                extent,
            });
        }
        self.status
            .lookup(iblock)
            .map(|run| run.clip(u64::from(iblock), end))
    }

    pub(super) fn record(&mut self, run: ExtentRun) {
        if let ExtentRun::Mapped { extent, .. } = &run {
            self.last.record(*extent);
        }
        self.status.record(run);
    }

    pub(super) fn covers_initialized(&self, range: Range<u64>) -> bool {
        self.status.covers_initialized(range)
    }

    pub(super) fn invalidate(&mut self, range: Range<u64>) {
        self.status.invalidate(range.clone());
        self.last.invalidate(range);
    }
}

/// Reconstructible logical mapping facts. A missing entry means unknown,
/// never a hole. Overlapping edits discard the entire old fact conservatively.
#[derive(Debug, Default)]
pub(super) struct ExtentStatusTree {
    entries: BTreeMap<u64, ExtentRun>,
}

/// One validated extent, invalidated with the logical status index on edits.
#[derive(Debug, Default)]
struct LastExtent(Option<Extent>);

impl ExtentStatusTree {
    pub(super) fn lookup(&self, iblock: Iblock) -> Option<ExtentRun> {
        let start = u64::from(iblock);
        self.entries
            .range(..=start)
            .next_back()
            .filter(|(_, run)| run.bounds().contains(&start))
            .map(|(_, run)| run.clone())
    }

    pub(super) fn record(&mut self, run: ExtentRun) {
        if run.bounds().is_empty() {
            return;
        }
        self.invalidate(run.bounds().clone());
        if self.entries.len() >= MAX_STATUS_ENTRIES {
            self.entries.pop_first();
        }
        self.entries.insert(run.bounds().start, run);
    }

    pub(super) fn invalidate(&mut self, range: Range<u64>) {
        let mut keys: Vec<_> = self
            .entries
            .range(range.clone())
            .map(|(&key, _)| key)
            .collect();
        if let Some((&key, run)) = self.entries.range(..range.start).next_back()
            && run.bounds().end > range.start
        {
            keys.push(key);
        }
        for key in keys {
            self.entries.remove(&key);
        }
    }

    pub(super) fn covers_initialized(&self, range: Range<u64>) -> bool {
        let mut start = range.start;
        while start < range.end {
            let Some(ExtentRun::Mapped { logical, extent }) = self.lookup(start as Iblock) else {
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
    pub(super) fn lookup(&self, iblock: Iblock) -> Option<Extent> {
        self.0.filter(|e| e.covers(iblock))
    }
    pub(super) fn record(&mut self, extent: Extent) {
        self.0 = Some(extent);
    }
    pub(super) fn invalidate(&mut self, range: Range<u64>) {
        if self
            .0
            .is_some_and(|e| u64::from(e.block()) < range.end && e.logical_end() > range.start)
        {
            self.0 = None;
        }
    }
}

const MAX_STATUS_ENTRIES: usize = 512;
