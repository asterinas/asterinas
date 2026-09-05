// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! Inode identity-reuse cache of the overlay inode module.
//!
//! [`InodeCache`] maps each real-object identity ([`RealObjectKey`]) to the
//! shared [`OverlayInode`]. The identity-reuse invariant holds: while any
//! reference to an overlay inode lives, every lookup that resolves the same
//! real object (same `fsid`, same real inode number) reuses the same inode
//! instead of constructing a duplicate one. (This is what keeps hard-linked
//! names converged on one overlay inode.)

use core::{
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use hashbrown::HashMap;

use crate::{
    fs::{
        fs_impls::overlayfs::{inode::OverlayInode, real::RealObjectKey},
        vfs::inode::Inode,
    },
    prelude::*,
};

const SWEEP_INTERVAL: u64 = 1024;

#[derive(Clone)]
struct InodeCacheEntry {
    carrier: Weak<OverlayInode>,
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

/// After copy-up rekeys an entry to its new upper key (`replace_facts` /
/// `publish_rekey`), the entry is also retained under the old key as a
/// stale alias; the periodic sweep in `get_or_create` retires it once its
/// overlay inode has no strong references left.
#[derive(Debug)]
pub(in overlayfs) struct InodeCache {
    entries: RwMutex<HashMap<RealObjectKey, InodeCacheEntry>>,
    misses_since_sweep: AtomicU64,
}

impl InodeCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            entries: RwMutex::new(HashMap::new()),
            misses_since_sweep: AtomicU64::new(0),
        }
    }

    pub(super) fn get(&self, key: RealObjectKey) -> Option<Arc<OverlayInode>> {
        self.entries
            .read()
            .get(&key)
            .and_then(|entry| entry.carrier.upgrade())
    }

    /// Infallible by construction: the caller passes in the committing
    /// `pin` instead of reading it from the old-key slot (another task may
    /// transiently have placed a freshly created sibling inode there), and
    /// both inserts supersede any occupant rather than fail.
    pub(super) fn publish_rekey(
        &self,
        old_key: RealObjectKey,
        new_key: RealObjectKey,
        old_real_inode: Arc<dyn Inode>,
        pin: &Arc<OverlayInode>,
    ) {
        let mut guard = self.entries.write();
        if let Some(existing) = guard.get(&new_key)
            && existing.carrier.strong_count() > 0
            && !Weak::ptr_eq(&existing.carrier, &Arc::downgrade(pin))
            && let Some(occupant) = existing.carrier.upgrade()
        {
            if occupant.contains_real_inode(pin.visible_source().real_inode()) {
                notice!(
                    "overlay inode-cache convergence at the post-transition key \
                     {:?}: an early projection of the same real object is \
                     superseded by the committing inode",
                    new_key
                );
            } else {
                error!(
                    "overlay inode-cache stale identity at the post-transition key \
                     {:?}: replacing the occupant with the committing inode \
                     (ino reuse)",
                    new_key
                );
            }
            // The occupant may die after this check; the insert below
            // supersedes the entry regardless, and a superseded entry with
            // no strong references is retired by the periodic sweep.
        }
        guard.insert(
            new_key,
            InodeCacheEntry {
                carrier: Arc::downgrade(pin),
                keep_alive: None,
            },
        );
        if new_key != old_key {
            guard.insert(
                old_key,
                InodeCacheEntry {
                    carrier: Arc::downgrade(pin),
                    keep_alive: Some(old_real_inode),
                },
            );
        }
    }

    /// A live hit is validated by `is_same_object`; a stale (ino-reuse)
    /// occupant is evicted so a key never serves a different real object.
    pub(super) fn get_or_create(
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
        guard.remove(&key);
        let misses = self.misses_since_sweep.fetch_add(1, Ordering::Relaxed) + 1;
        if misses.is_multiple_of(SWEEP_INTERVAL) {
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
