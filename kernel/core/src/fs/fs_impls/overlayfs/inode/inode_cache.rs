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
    fs::fs_impls::overlayfs::{
        inode::{OverlayInode, identity::ObjectRealId},
        real::RealObject,
    },
    prelude::*,
};

const SWEEP_INTERVAL: u64 = 1024;

/// The mount-wide stable-identity map: a dead or mismatched entry is replaced on the next miss.
///
/// The three operations split the work by lock strength: [`InodeCache::reusable_occupant`] probes
/// with a read guard alone, while [`InodeCache::register`] and [`InodeCache::rekey`] hold the
/// write guard: `register` replaces an entry and sweeps the dead ones out, and `rekey` moves one
/// entry onto the key its instance's identity now resolves to.
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

    /// Absent, dead, and rejected occupants all report `None`, so the caller registers.
    pub(super) fn reusable_occupant(
        &self,
        real_id: ObjectRealId,
        upper: Option<&RealObject>,
        topmost_lower: Option<&RealObject>,
    ) -> Option<Arc<OverlayInode>> {
        let guard = self.entries.upread();
        let inode = guard.get(&real_id).and_then(Weak::upgrade)?;
        if inode.is_reusable_for(upper, topmost_lower) {
            return Some(inode);
        }
        error!(
            "overlay inode-cache stale identity at key {:?}: the cached inode no \
             longer denotes the same real object; replacing it",
            real_id
        );
        None
    }

    /// A live reusable occupant wins over `instance`; the sweep drops dead entries.
    pub(super) fn register(
        &self,
        real_id: ObjectRealId,
        instance: Arc<OverlayInode>,
    ) -> Arc<OverlayInode> {
        let mut guard = self.entries.upread().upgrade();
        // Registration never displaces a live reusable occupant.
        if let Some(occupant) = guard.get(&real_id).and_then(Weak::upgrade)
            && occupant.is_reusable_for(instance.upper.get(), instance.lowers.first())
        {
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

    /// The move is qualified by what the map holds: the entry at `old_key` goes only when it
    /// points to `instance`, and an instance the map does not hold stays unregistered. `new_key`
    /// then serves `instance`, which is the only entry that key can hold: it names the upper real
    /// object this instance just published, and no reusable occupant can be there — one projected
    /// from that object alone retains no lower for `is_reusable_for` to match, and one left by a
    /// recycled inode number denotes a different real object.
    pub(super) fn rekey(
        &self,
        old_key: ObjectRealId,
        new_key: ObjectRealId,
        instance: &Arc<OverlayInode>,
    ) {
        let mut guard = self.entries.upread().upgrade();
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
