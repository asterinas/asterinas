// SPDX-License-Identifier: MPL-2.0

//! Inode identity-reuse cache of the overlay inode module.
//!
//! [`InodeCache`] maps each stable identity key ([`ObjectRealId`]) to the
//! shared [`OverlayInode`]. The identity-reuse invariant holds: while any
//! reference to an overlay inode lives, every name that resolves the same
//! stable identity (same `fsid`, same real inode number) reuses the same inode
//! instead of constructing a duplicate one. (This is what keeps hard-linked
//! names converged on one overlay inode.) A copy-up that persists no durable
//! origin record therefore *moves* the promoted instance's entry onto the key
//! that instance's identity resolves to from then on ([`InodeCache::rekey`]),
//! so a name resolved after the promotion hits the live instance instead of
//! projecting a second one. One degradation is accepted: the two names of one
//! lower non-directory hard link each hold their own instance, a pair this
//! cache holds aside so that promoting one name leaves its sibling's view
//! untouched. The identity-reuse invariant therefore covers the objects this
//! cache holds.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicU64, Ordering};

use hashbrown::HashMap;

use crate::{
    fs::fs_impls::overlayfs::inode::{OverlayInode, identity::ObjectRealId},
    prelude::*,
};

/// The mount-wide stable-identity map.
///
/// A dead entry is replaced on the next miss. [`InodeCache::occupant`] probes with a read guard
/// alone, which is the fast path a repeat lookup takes. [`InodeCache::register`] replaces an entry
/// and sweeps the dead ones out, and [`InodeCache::rekey`] moves one entry onto the key its
/// instance's identity now resolves to; both hold the write guard.
#[derive(Debug)]
pub(in overlayfs) struct InodeCache {
    entries: RwMutex<HashMap<ObjectRealId, Weak<OverlayInode>>>,
    misses_since_sweep: AtomicU64,
}

impl InodeCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            entries: RwMutex::new(HashMap::new()),
            misses_since_sweep: AtomicU64::new(0),
        }
    }

    /// Returns the live occupant of `real_id`, if the map holds one.
    pub(super) fn occupant(&self, real_id: ObjectRealId) -> Option<Arc<OverlayInode>> {
        let guard = self.entries.read();
        guard.get(&real_id).and_then(Weak::upgrade)
    }

    /// Registers `instance` under `real_id`, returning the live occupant the key already has.
    ///
    /// A registration that lands also counts a miss, and the sweep drops the dead entries every
    /// `SWEEP_INTERVAL` misses.
    pub(super) fn register(
        &self,
        real_id: ObjectRealId,
        instance: Arc<OverlayInode>,
    ) -> Arc<OverlayInode> {
        /// How many registration misses pass between two sweeps of the dead entries.
        const SWEEP_INTERVAL: u64 = 1024;
        let mut guard = self.entries.write();
        // Registration never displaces a live occupant.
        if let Some(occupant) = guard.get(&real_id).and_then(Weak::upgrade) {
            return occupant;
        }
        guard.remove(&real_id);
        let misses = self.misses_since_sweep.fetch_add(1, Ordering::Relaxed) + 1;
        if misses.is_multiple_of(SWEEP_INTERVAL) {
            guard.retain(|_, entry| entry.strong_count() > 0);
        }
        guard.insert(real_id, Arc::downgrade(&instance));
        instance
    }

    /// Moves `instance`'s entry from `old_key` onto the key its identity now resolves to.
    ///
    /// The move is qualified by what the map holds: the entry at `old_key` goes only when it points
    /// to `instance`, and `new_key` can hold no other entry, because it names the upper real object
    /// this instance just published.
    pub(super) fn rekey(
        &self,
        old_key: ObjectRealId,
        new_key: ObjectRealId,
        instance: &Arc<OverlayInode>,
    ) {
        let mut guard = self.entries.write();
        let is_own_entry = guard
            .get(&old_key)
            .and_then(Weak::upgrade)
            .is_some_and(|occupant| Arc::ptr_eq(&occupant, instance));
        if !is_own_entry {
            return;
        }
        guard.remove(&old_key);
        guard.insert(new_key, Arc::downgrade(instance));
    }
}
