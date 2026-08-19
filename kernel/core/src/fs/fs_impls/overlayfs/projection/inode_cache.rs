// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Inode identity-reuse cache of the overlay projection.
//!
//! This module owns the [`RealObjectKey`] identity pair and the mount-wide
//! [`InodeCache`] that maps each real-object identity to the shared
//! [`OverlayInode`]. The hard-link invariant holds: while any reference to an
//! overlay inode lives, every lookup that resolves the same real object
//! (same `fsid`, same real inode number) reuses the same inode instead of
//! constructing a duplicate one.
//!
//! # Structure
//!
//! | Item | Owns |
//! |---|---|
//! | [`RealObjectKey`] | The `(fsid, real ino)` identity pair. |
//! | [`InodeCache`] | The mount-wide identity-reuse map. |
//! | `InodeCacheEntry` | One cache entry (weak inode pin + optional keep-alive). |

use core::{
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use hashbrown::HashMap;

use super::lookup::RealObject;
use crate::{
    fs::{
        fs_impls::overlayfs::inode::{ObjectFacts, OverlayInode},
        vfs::inode::Inode,
    },
    prelude::*,
};

const SWEEP_INTERVAL: u64 = 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in overlayfs) struct RealObjectKey {
    /// Layer fsid of the visible-metadata source (upper, else topmost lower).
    fsid: u64,
    /// Real inode number of the visible-metadata source.
    real_ino: u64,
}

impl RealObjectKey {
    pub(in overlayfs) fn from_source(real: &RealObject) -> Self {
        Self {
            fsid: real.fsid(),
            real_ino: real.real_inode().ino(),
        }
    }

    pub(in overlayfs) fn from_facts(facts: &ObjectFacts) -> Self {
        Self::from_source(facts.visible_source())
    }
}

#[derive(Clone)]
struct InodeCacheEntry {
    /// Weak pin to the shared [`OverlayInode`].
    carrier: Weak<OverlayInode>,
    /// Strong keep-alive of the real inode denoted by this entry's key when
    /// the inode's facts no longer pin it (stale alias); `None` otherwise.
    keep_alive: Option<Arc<dyn Inode>>,
}

impl Debug for InodeCacheEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InodeCacheEntry")
            .field("carrier", &self.carrier)
            .field(
                "keep_alive",
                &self.keep_alive.as_ref().map(|_| "<real-inode keep-alive>"),
            )
            .finish()
    }
}

/// The mount-wide inode identity-reuse cache.
///
/// Invariants: one real object maps to one [`OverlayInode`] while any
/// reference lives. After a copy-up facts transition the inode is also
/// registered under a retained old-key alias, retired by the dead-pin sweep
/// once the inode drops.
#[derive(Debug)]
pub(in overlayfs) struct InodeCache {
    /// Weak inode pins (with optional stale-alias keep-alives).
    entries: RwMutex<HashMap<RealObjectKey, InodeCacheEntry>>,
    /// Miss-path insert counter driving the `SWEEP_INTERVAL`-based dead-entry
    /// sweep.
    misses_since_sweep: AtomicU64,
}

impl InodeCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            entries: RwMutex::new(HashMap::new()),
            misses_since_sweep: AtomicU64::new(0),
        }
    }

    /// Returns the cached overlay inode for `key`, if a live inode is
    /// registered.
    pub(in overlayfs) fn get(&self, key: RealObjectKey) -> Option<Arc<OverlayInode>> {
        self.entries
            .read()
            .get(&key)
            .and_then(|entry| entry.carrier.upgrade())
    }

    /// Aliases an inode's cache registration under `new_key` while retaining
    /// the `old_key` mapping.
    ///
    /// Both keys resolve to the same inode pin, so a concurrent in-flight
    /// projection cannot mint or orphan a second inode. A live occupant at
    /// `new_key` for a different inode is either a displaced concurrent
    /// projection (`Err`, never silently clobbered) or an ino-reuse stale
    /// occupant that is replaced.
    pub(in overlayfs) fn rekey_keep_old_alias(
        &self,
        old_key: RealObjectKey,
        new_key: RealObjectKey,
        old_real_inode: Arc<dyn Inode>,
        new_visible_source: &RealObject,
    ) -> Result<()> {
        let mut guard = self.entries.write();
        let Some(old_entry) = guard.get(&old_key).cloned() else {
            // Nothing registered under the pre-transition key (already
            // aliased or never registered): no-op.
            return Ok(());
        };
        if old_entry.carrier.strong_count() == 0 {
            // Dead pre-transition pin: retire it; there is nothing to alias.
            guard.remove(&old_key);
            return Ok(());
        }
        match guard.get(&new_key) {
            // `new_key` already maps to a live pin of a DIFFERENT inode:
            // clobbering it would silently orphan that inode.
            Some(existing)
                if existing.carrier.strong_count() > 0
                    && !Weak::ptr_eq(&existing.carrier, &old_entry.carrier) =>
            {
                let Some(existing_carrier) = existing.carrier.upgrade() else {
                    return Err(Error::with_message(
                        Errno::EIO,
                        "the overlay inode-cache occupant disappeared during the alias transition",
                    ));
                };
                if existing_carrier
                    .facts_snapshot()
                    .contains_real_inode(new_visible_source.real_inode())
                {
                    // The same real object was projected early under the new key by
                    // a concurrent lookup; keep the displacement error instead of
                    // silently reusing it.
                    error!(
                        "overlay inode-cache displacement: a live inode for the SAME real \
                         object is already registered at the post-transition key {:?}; the \
                         copy-up transition must fail",
                        new_key
                    );
                    Err(Error::with_message(
                        Errno::EIO,
                        "the overlay inode cache already maps the new visible-source key to the same real object",
                    ))
                } else {
                    // A different object (ino reuse) stale-occupies the new key; replace
                    // that entry with this inode and keep the old-key alias.
                    error!(
                        "overlay inode-cache stale identity at the post-transition key {:?}: \
                         replacing the occupant with the transitioning inode (ino reuse)",
                        new_key
                    );
                    guard.insert(
                        old_key,
                        InodeCacheEntry {
                            carrier: old_entry.carrier.clone(),
                            keep_alive: Some(old_real_inode),
                        },
                    );
                    guard.insert(
                        new_key,
                        InodeCacheEntry {
                            carrier: old_entry.carrier,
                            keep_alive: None,
                        },
                    );
                    Ok(())
                }
            }
            // `new_key` is empty, holds a dead pin, or already aliases the
            // same inode (idempotent re-alias): publish the alias.
            _ => {
                let carrier = old_entry.carrier;
                guard.insert(
                    old_key,
                    InodeCacheEntry {
                        carrier: carrier.clone(),
                        keep_alive: Some(old_real_inode),
                    },
                );
                if new_key != old_key {
                    guard.insert(
                        new_key,
                        InodeCacheEntry {
                            carrier,
                            keep_alive: None,
                        },
                    );
                }
                Ok(())
            }
        }
    }

    /// Returns the cached overlay inode for `key`, or creates and publishes
    /// one via `create_fn` on a miss.
    ///
    /// On a live hit, `is_same_object` validates the cached inode; a stale
    /// inode (backing-fs ino reuse) is evicted and replaced so the key is
    /// never served a different real object. The check-then-publish sequence
    /// is atomic.
    pub(in overlayfs) fn get_or_create(
        &self,
        key: RealObjectKey,
        is_same_object: impl FnOnce(&Arc<OverlayInode>) -> bool,
        create_fn: impl FnOnce() -> Arc<OverlayInode>,
    ) -> Arc<OverlayInode> {
        let guard = self.entries.upread();
        if let Some(inode) = guard.get(&key).and_then(|entry| entry.carrier.upgrade()) {
            if is_same_object(&inode) {
                return inode;
            }
            error!(
                "overlay inode-cache stale identity at key {:?}: the cached inode no \
                 longer denotes the same real object (ino reuse); replacing it",
                key
            );
        }
        let inode = create_fn();
        let mut guard = guard.upgrade();
        // O(1) per-key eviction: clears any stale weak pin before the fresh
        // inode replaces it.
        guard.remove(&key);
        // Amortized full sweep: every `SWEEP_INTERVAL`-th miss.
        let misses = self.misses_since_sweep.fetch_add(1, Ordering::Relaxed) + 1;
        if misses.is_multiple_of(SWEEP_INTERVAL) {
            // Reclaims dead inode pins AND their stale-alias keep-alives:
            // dropping the entry drops the keep-alive `Arc`.
            guard.retain(|_, entry| entry.carrier.strong_count() > 0);
        }
        guard.insert(
            key,
            InodeCacheEntry {
                carrier: Arc::downgrade(&inode),
                keep_alive: None,
            },
        );
        inode
    }
}
