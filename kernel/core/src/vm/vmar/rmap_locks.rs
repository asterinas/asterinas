// SPDX-License-Identifier: MPL-2.0

//! All-or-nothing reverse-map locking for forward mapping changes.
//!
//! A caller snapshots the backing objects under its page-table cursor and
//! tries to lock all of them before changing any mappings. On contention it
//! drops the cursor, allocator guards, and preemption guard before waiting,
//! then restarts the whole operation with a fresh snapshot. No partial syscall
//! changes are exposed merely because a reverse-map lock is busy.

use super::{Rmap, RmapEntry, Vmar};
use crate::{
    prelude::*,
    vm::{
        dmo::{Dmo, MapOperation, PendingDmoMapping},
        page_cache::Vmo,
    },
};

/// A backing object whose reverse mappings must be kept in sync with the PT.
#[derive(Clone)]
pub(super) enum RmapObject {
    Vmo(Arc<Vmo>),
    Dmo(Arc<Dmo>),
}

impl RmapObject {
    fn rmap(&self) -> &Mutex<Rmap> {
        match self {
            Self::Vmo(vmo) => vmo.rmap(),
            Self::Dmo(dmo) => dmo.rmap(),
        }
    }

    /// A stable identity while this object is kept alive by its Arc.
    pub(super) fn key(&self) -> usize {
        core::ptr::from_ref(self.rmap()) as usize
    }

    /// Waits for a contended lock, outside atomic mode and without other locks.
    pub(super) fn wait(&self) {
        drop(self.rmap().lock());
    }
}

/// The unique set of reverse maps affected by one forward operation.
pub(super) struct RmapTargets(BTreeMap<usize, RmapObject>);

impl RmapTargets {
    pub(super) fn new(entries: &[(RmapObject, RmapEntry)]) -> Self {
        let mut targets = Self(BTreeMap::new());
        for (object, _) in entries {
            targets.insert(object.clone());
        }
        targets
    }

    pub(super) fn insert(&mut self, object: RmapObject) {
        self.0.entry(object.key()).or_insert(object);
    }

    /// Tries to lock every target, releasing all acquired locks on failure.
    pub(super) fn try_lock(&self) -> Result<RmapLocks<'_>, RmapObject> {
        let mut locks = BTreeMap::new();
        for (key, object) in &self.0 {
            let Some(lock) = object.rmap().try_lock() else {
                return Err(object.clone());
            };
            locks.insert(*key, lock);
        }
        Ok(RmapLocks(locks))
    }
}

/// Reverse-map locks held from before the first PT change through publication.
pub(super) struct RmapLocks<'a>(BTreeMap<usize, MutexGuard<'a, Rmap>>);

impl RmapLocks<'_> {
    pub(super) fn take_pending(&mut self, pending: PendingDmoMapping) -> Vec<MapOperation> {
        let object = RmapObject::Dmo(pending.dmo().clone());
        let rmap = self
            .0
            .get_mut(&object.key())
            .expect("pending mapping's DMO must be locked before publication");
        pending.take(rmap)
    }

    pub(super) fn refresh(
        &mut self,
        vmar: Weak<Vmar>,
        new_entries: Vec<(RmapObject, RmapEntry)>,
        ranges: &[core::ops::Range<Vaddr>],
    ) {
        for rmap in self.0.values_mut() {
            for range in ranges {
                rmap.remove_range(vmar.clone(), range);
            }
        }
        for (object, entry) in new_entries {
            let rmap = self
                .0
                .get_mut(&object.key())
                .expect("new reverse mapping was not locked before the PT change");
            rmap.insert(vmar.clone(), entry);
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::{prelude::ktest, task};

    use super::*;

    #[ktest]
    fn contention_releases_all_locks_and_deduplicates_targets() {
        let first = Dmo::new();
        let busy = Dmo::new();
        let mut targets = RmapTargets::new(&[]);
        targets.insert(RmapObject::Dmo(first.clone()));
        targets.insert(RmapObject::Dmo(first.clone()));
        targets.insert(RmapObject::Dmo(busy.clone()));

        let busy_lock = busy.rmap().lock();
        let preempt_guard = task::disable_preempt();
        assert!(targets.try_lock().is_err());
        assert!(first.rmap().try_lock().is_some());
        drop(busy_lock);
        let locks = targets.try_lock().ok().unwrap();
        assert!(first.rmap().try_lock().is_none());
        assert!(busy.rmap().try_lock().is_none());
        drop(locks);
        drop(preempt_guard);
    }
}
