// SPDX-License-Identifier: MPL-2.0

//! Inode identity-reuse cache of the overlay projection.
//!
//! This module owns the [`RealObjectKey`] identity pair and the mount-wide
//! [`InodeCache`] that maps each real-object identity to the shared
//! [`OverlayInode`]. The hard-link invariant holds: while any
//! reference to an overlay inode lives, every lookup that resolves the same
//! real object (same `fsid`, same real inode number) reuses the same inode
//! instead of constructing a duplicate one.
//!
//! # Locking
//!
//! [`InodeCache`] is the mount-wide `OverlayFs::inodes` cache; its
//! `ostd::sync::RwMutex` is an internal data lock, and
//! [`InodeCache::get_or_create`] follows the VFS children-cache `upread` →
//! `upgrade` pattern so the check-then-publish sequence is atomic: a writer
//! cannot enter while an upgradeable reader is held, and the
//! upgradeable-reader slot is single, so concurrent creators for one key are
//! serialized. Values are [`InodeCacheEntry`]s — a weak [`OverlayInode`]
//! pin plus, for a retained stale-alias entry, a strong keep-alive pin
//! of the pre-transition real inode — so the cache never keeps an overlay
//! inode alive and never forms an `OverlayFs → OverlayInode → OverlayFs`
//! strong cycle.

use core::{
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use hashbrown::HashMap;

use super::{
    entry::RealObject,
    inode::{OverlayInode, OverlayObjectFacts},
    visible_source,
};
use crate::{fs::vfs::inode::Inode, prelude::*};

/// Full-map dead-entry sweep interval: one O(live) sweep per this many
/// miss-path inserts keeps dead `Weak` accumulation bounded with O(1)
/// amortized cost on the per-path-component lookup hot path.
const SWEEP_INTERVAL: u64 = 1024;

/// The identity of the real object that is the visible-metadata source of an
/// overlay inode.
///
/// The pair is the layer `fsid` of the visible-metadata source (upper, else
/// topmost lower) and that source's real inode number. Hard links to the same
/// real object collapse onto one key, and merged directories key on their
/// visible-metadata source; there is deliberately no `ID -> name` reverse map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct RealObjectKey {
    /// Layer fsid of the visible-metadata source (upper, else topmost lower).
    fsid: u64,
    /// Real inode number of the visible-metadata source.
    real_ino: u64,
}

impl RealObjectKey {
    /// Builds the identity-reuse key from the visible-metadata source.
    ///
    /// Returns the `(fsid, real_inode.ino())` pair of `real`; merged
    /// directories are keyed by their visible-metadata source.
    pub(super) fn from_source(real: &RealObject) -> Self {
        Self {
            fsid: real.fsid(),
            real_ino: real.real_inode().ino(),
        }
    }

    /// Builds the identity-reuse key from the visible-metadata source of
    /// `facts`.
    ///
    /// The visible-source selection and the key derivation are the two halves
    /// of one identity rule (the `lowers[0]`-indexing selection centralized
    /// in `visible_source`), so all callers derive the key through this
    /// single path instead of repeating `from_source(visible_source(..))`.
    pub(super) fn from_facts(facts: &OverlayObjectFacts) -> Self {
        Self::from_source(visible_source(facts))
    }
}

/// One inode-cache entry: a weak pin to the shared [`OverlayInode`] plus, for
/// a retained stale alias, a strong keep-alive pin of the pre-transition
/// visible-source real inode.
///
/// The keep-alive is `Some` only on the retained old-key mapping created by
/// [`InodeCache::alias_key`]: it pins the real inode whose identity the stale
/// key denotes, so the underlying inode cannot be recycled (ino-reuse) while
/// the stale alias exists. It is dropped when the dead-pin sweep reclaims the
/// entry (the inode weak has died), bounding its lifetime to the inode's.
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
/// Invariants: one real object → one [`OverlayInode`] while any
/// reference lives; merged directories key on their visible-metadata source;
/// no `ID -> name` reverse map exists. An inode is registered under its
/// visible-source key and — after a copy-up facts transition — also under a
/// retained old-key alias (`alias_key` aliases instead of moving, so both keys
/// resolve to the one inode; the dead-pin sweep retires the old alias once
/// the inode drops). The retained alias also carries a strong keep-alive pin
/// of the pre-transition real inode, so the underlying inode cannot be
/// recycled while the alias exists. Values are `Weak` inode pins plus an
/// optional keep-alive, so the cache never keeps an overlay inode alive by
/// itself.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct InodeCache {
    /// Weak inode pins (with optional stale-alias keep-alives); no
    /// `OverlayFs → OverlayInode → OverlayFs` cycle.
    by_key: RwMutex<HashMap<RealObjectKey, InodeCacheEntry>>,
    /// Miss-path insert counter driving the amortized dead-entry sweep: a full
    /// sweep runs every `SWEEP_INTERVAL` misses, keeping the bounded-memory
    /// property with O(1) amortized cost instead of an O(live) sweep on every
    /// miss.
    misses_since_sweep: AtomicU64,
}

impl InodeCache {
    /// Constructs an empty cache (`mount/build.rs` initializes the field
    /// through this constructor instead of a struct literal over private
    /// fields).
    pub(in crate::fs::fs_impls::overlayfs) fn new() -> Self {
        Self {
            by_key: RwMutex::new(HashMap::new()),
            misses_since_sweep: AtomicU64::new(0),
        }
    }

    /// Returns the cached overlay inode for `key`, if a live inode is
    /// registered.
    ///
    /// A read-only probe that holds no upgradeable-reader slot, so it never
    /// re-enters the inode cache's single upgradeable-reader slot.
    /// `OverlayFs::publication_parent` uses this probe to resolve the live
    /// parent inode; a miss is treated as an invariant violation
    /// (debug-assert + error log + `Err`), not followed by a projection.
    pub(super) fn get(&self, key: RealObjectKey) -> Option<Arc<OverlayInode>> {
        self.by_key
            .read()
            .get(&key)
            .and_then(|entry| entry.carrier.upgrade())
    }

    /// Aliases an inode's cache registration under `new_key` while retaining
    /// the `old_key` mapping.
    ///
    /// The copy-up facts transition changes the visible-source key of an
    /// already-registered inode. This method adds the new-key mapping to
    /// the SAME inode pin instead of moving it, so both keys resolve to one
    /// inode and a concurrent in-flight projection cannot mint or orphan a
    /// second inode: a lookup that captured the pre-transition (stale)
    /// facts still hits the retained `old_key` alias through `get_or_create`
    /// and reuses the one inode, and a lookup that already observed the new
    /// source resolves through the new mapping. The old alias is a `Weak`
    /// inode pin (it never keeps the inode alive) and is retired by the
    /// amortized dead-pin sweep once the inode drops.
    ///
    /// `new_visible_source` is the post-transition visible source of the
    /// inode's new facts (the upper real object the copy-up published). It
    /// tells the two live-occupant branches at `new_key` apart. When a live
    /// pin for a DIFFERENT inode already exists there and that occupant's
    /// facts contain the new visible source, a concurrent early projection
    /// of the SAME real upper displaced this transition: the registration is
    /// never silently clobbered — the displacement is returned as `Err`,
    /// logged and detectable — so the copy-up caller can fail or retry the
    /// transition instead of proceeding with a split. When the occupant's
    /// facts do NOT contain the new visible source (an ino-reuse stale
    /// occupant), the occupant is replaced by this inode and the `old_key`
    /// alias retained, so the transition self-heals instead of failing.
    /// Copy-up must serialize the transition against projections of the
    /// transitioning object (e.g. hold the object's and the parents' `DIR`
    /// transactions across `replace_facts`).
    ///
    /// # Stale-alias real-inode keep-alive
    ///
    /// `old_real_inode` is the pre-transition visible-source real inode (the
    /// inode's old facts pin it; after the transition the inode's facts
    /// may drop it). The retained `old_key` entry stores it as a strong
    /// keep-alive alongside the weak inode, so the underlying inode cannot
    /// be recycled (ino-reuse) while the stale alias exists — a lookup
    /// resolving the stale key can only ever return the same inode for the
    /// same real object. The keep-alive is dropped when the dead-pin sweep
    /// reclaims the entry (the inode weak has died), bounding its lifetime
    /// to the inode's. The `new_key` entry needs no keep-alive: the
    /// post-transition visible source is pinned by the inode's own facts.
    /// Resolution semantics stay correct across whiteout/reversion: a
    /// reverted or whiteout-hidden name resolves through the overlay's own
    /// whiteout/binding evidence before any stale-key identity is consulted,
    /// and the alias always maps the old key to the same logical object that
    /// published it.
    pub(super) fn alias_key(
        &self,
        old_key: RealObjectKey,
        new_key: RealObjectKey,
        old_real_inode: Arc<dyn Inode>,
        new_visible_source: &RealObject,
    ) -> Result<()> {
        let mut guard = self.by_key.write();
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
                // Defensive upgrade: `strong_count() > 0` implies the upgrade
                // succeeds; a `None` here is logically unreachable and degrades to
                // the displacement error (no `.unwrap()`/`.expect()`).
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
            // SAME inode (idempotent re-alias): publish the alias, and pin
            // the pre-transition real inode on the retained old-key entry so
            // it cannot be recycled while the stale alias exists.
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
    /// On a live hit, `is_same_object` runs as a brief validation under the
    /// upgradeable read guard: it must take only the cached inode's facts
    /// snapshot and must not acquire any other overlay lock. When the cached
    /// inode still denotes the same logical object it is returned unchanged
    /// (reuse); when it no longer does (backing-fs inode reuse), the stale
    /// inode is evicted and the create path publishes the fresh inode in
    /// its place, so the key is never served an inode for a different real
    /// object.
    ///
    /// The check-then-publish sequence is atomic (`upread` → `upgrade`): while
    /// the upgradeable read guard is held no writer can publish another
    /// inode for the same key, and the single upgradeable-reader slot
    /// serializes concurrent creators, so exactly one inode per key is ever
    /// published. A stale `Weak` entry whose inode has been dropped is evicted
    /// per-key in O(1), and an amortized full sweep every `SWEEP_INTERVAL`
    /// misses keeps the bounded-memory property without per-miss linear cost.
    pub(super) fn get_or_create(
        &self,
        key: RealObjectKey,
        is_same_object: impl FnOnce(&Arc<OverlayInode>) -> bool,
        create_fn: impl FnOnce() -> Arc<OverlayInode>,
    ) -> Arc<OverlayInode> {
        let guard = self.by_key.upread();
        if let Some(inode) = guard.get(&key).and_then(|entry| entry.carrier.upgrade()) {
            if is_same_object(&inode) {
                return inode;
            }
            error!(
                "overlay inode-cache stale identity at key {:?}: the cached inode no \
                 longer denotes the same real object (ino reuse); replacing it",
                key
            );
            // Fall through to the create path: `upgrade` then `remove(key)`
            // then insert the fresh inode (the existing miss-path body).
        }
        let inode = create_fn();
        let mut guard = guard.upgrade();
        // O(1) per-key eviction: the single upgradeable-reader slot guarantees
        // no writer published this key between the miss read and the upgrade,
        // so `remove` is a no-op when the key was absent and otherwise clears
        // the stale weak pin before the fresh inode replaces it.
        guard.remove(&key);
        // Amortized full sweep: every `SWEEP_INTERVAL`-th miss, evict the
        // whole map's dead weak pins under the same write guard (O(live) but
        // only once per interval — O(1) amortized per miss).
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
