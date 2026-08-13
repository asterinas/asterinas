// SPDX-License-Identifier: MPL-2.0

//! The module root of the overlayfs projection and identity subsystem.
//!
//! This module declares the six `projection/*` submodules and hosts the
//! `OverlayFs`-extension impl blocks: the `bindings()`/`inodes()`/`identity()`
//! field accessors and the `lookup_binding`/`project_inode`/`publish_binding`
//! lookup orchestration (caller-side projection). The shared types are
//! re-exported so consumers in sibling module trees (`mount::build` step 10,
//! `mount::superblock` field types, and the leaf modules
//! `readdir_index.rs`/`copyup/`/`dir/`) can name them: [`OverlayInode`],
//! [`OverlayObjectFacts`], [`BindingCache`], [`InodeCache`],
//! [`IdentityPolicy`], and the wider type set — [`Binding`],
//! [`BindingKey`], [`PositiveBinding`], [`NegativeBinding`],
//! [`HiddenEvidence`], [`PositiveKind`], [`RealObject`], [`RealObjectKey`],
//! and [`OverlayObjectId`](identity::OverlayObjectId). Only the module-private intermediate
//! (`LayerLookup`) and the `pub(super)` fields stay reachable only
//! inside this module tree.
//!
//! The cross-module helpers `OverlayFs::project_new_upper` and the
//! `record_copyup_transition` invocation at the positive-binding assembly
//! point are declared here; their method bodies live in the `copyup` and `dir`
//! modules.

mod binding_cache;
mod entry;
mod identity;
mod inode;
mod inode_cache;
mod lower_id;

pub(in crate::fs::fs_impls::overlayfs) use binding_cache::{
    Binding, BindingCache, BindingKey, HiddenEvidence, NegativeBinding, PositiveBinding,
    PositiveKind,
};
use entry::LayerLookup;
pub(in crate::fs::fs_impls::overlayfs) use entry::{RealObject, is_whiteout_inode};
pub(in crate::fs::fs_impls::overlayfs) use identity::{IdentityPolicy, LowerLayerIdentity};
pub(in crate::fs::fs_impls::overlayfs) use inode::{OverlayInode, OverlayObjectFacts};
pub(in crate::fs::fs_impls::overlayfs) use inode_cache::{InodeCache, RealObjectKey};

use super::mount::OverlayFs;
use crate::{
    fs::{fs_impls::overlayfs::readdir_index::ReaddirIndex, vfs::inode::Extension},
    prelude::*,
};

/// The result of one `(parent_id, name)` lookup: the binding to serve plus
/// the stale-upper signal.
///
/// `is_stale_upper` is true when the fresh layer truth no longer contains the
/// upper entry of a previously published upper-backed positive binding and no
/// whiteout covers the name. Most consumers ignore the signal (the binding is
/// already rebuilt from the fresh truth); the remove path consumes it to
/// surface `ESTALE` instead of re-exposing the lower counterpart.
pub(in crate::fs::fs_impls::overlayfs) struct LookupOutcome {
    /// The binding to serve: a verified cached binding, or the freshly
    /// rebuilt binding from the layer truth.
    pub(in crate::fs::fs_impls::overlayfs) binding: Binding,
    /// Whether this lookup observed the stale-upper class.
    pub(in crate::fs::fs_impls::overlayfs) is_stale_upper: bool,
}

impl OverlayFs {
    /// Returns the mount-wide binding cache.
    ///
    /// The cache is the first source for `(parent, name)` lookup results
    /// (`Binding-first` invariant); its internal `RwMutex` is an internal data
    /// lock used sequentially under the caller's parent `DIR` transaction.
    pub(super) fn bindings(&self) -> &BindingCache {
        &self.bindings
    }

    /// Returns the mount-wide inode identity-reuse cache.
    pub(super) fn inodes(&self) -> &InodeCache {
        &self.inodes
    }

    /// Returns the immutable identity policy of this mount.
    pub(super) fn identity(&self) -> &IdentityPolicy {
        &self.identity
    }

    /// Resolves one `name` under `parent_facts` into a [`LookupOutcome`]:
    /// the binding to serve plus the stale-upper signal.
    ///
    /// Called from `OverlayInode::lookup` (inode.rs) and the `dir/` recipes
    /// under the parent `DIR` transaction lock. The flow is verify-then-serve:
    /// the layer-ordered lookup (`lookup_in_layers`, entry.rs) runs
    /// unconditionally to re-observe the fresh layer truth; a cached
    /// `(parent_id, name)` binding is served only when it matches that truth
    /// (`matches_truth`); on a miss or a stale mismatch the binding is
    /// rebuilt from the fresh truth — the positive branch is projected into a
    /// shared [`OverlayInode`] (`project_inode`) — and the assembled binding
    /// is published before `DIR` release (`publish_binding`, which replaces
    /// the stale `Arc<Binding>` on mismatch). Negative variants stay private
    /// evidence and surface as `ENOENT` at the caller.
    ///
    /// The binding's `is_stale_upper` signal is derived on the mismatch
    /// branch only (`Binding::is_stale_upper`): a previously published
    /// upper-backed positive binding whose physical upper object vanished
    /// behind the overlay with no whiteout left. The binding is still rebuilt
    /// from the fresh truth (the next operation observes the honest truth);
    /// the remove path consumes the signal to surface `ESTALE` instead of
    /// re-exposing the lower counterpart; every other consumer ignores it.
    pub(super) fn lookup_binding(
        &self,
        parent_facts: &OverlayObjectFacts,
        name: &str,
    ) -> Result<LookupOutcome> {
        let parent_id = RealObjectKey::from_facts(parent_facts);
        // Fresh layer truth first: the memo is verified before it is served.
        let truth = self.lookup_in_layers(parent_facts, name)?;
        // Probe the cache: a cached binding is served ONLY when it matches the
        // fresh truth; a stale/mismatched hit falls through to the rebuild.
        // The `BindingCache` read guard is a temporary in the `if let`
        // condition and drops before `project_inode` runs below.
        let is_stale_upper = if let Some(binding) = self.bindings().get(&parent_id, name) {
            if binding.matches_truth(&truth) {
                return Ok(LookupOutcome {
                    binding: binding.as_ref().clone(),
                    is_stale_upper: false,
                });
            }
            // The cached binding mismatches the fresh truth: derive the
            // stale-upper signal on this mismatch branch only (same temporary
            // read guard; the brief `INODE` facts snapshot lives inside the
            // predicate and drops before any write).
            binding.is_stale_upper(&truth)
        } else {
            false
        };
        // Rebuild from the fresh truth.
        let binding = match truth {
            LayerLookup::Positive(facts) => {
                let inode = self.project_inode(&facts);
                Binding::Positive(PositiveBinding::new(inode))
            }
            LayerLookup::Negative(negative) => Binding::Negative(negative),
        };
        // Copy-up hook: at the positive-binding assembly point (the assembled
        // binding is published below via `publish_binding` before `DIR`
        // release), record the copy-up transition coordinate —
        // `publication_parent` + `name` — on the projected inode. The
        // once-per-inode guard (first positive binding wins; `try_lock`, skip
        // when contended) lives in the method body (`copyup/mod.rs`), so
        // re-publications after a binding invalidation are no-ops.
        if let Binding::Positive(positive) = &binding {
            // `publication_parent` resolves the live parent inode by its
            // visible-source key (the fallible copy-up transition aliases the
            // registration first — the old-key mapping is retained until the
            // dead-pin sweep reclaims it — so the probe cannot miss for a
            // live parent). A miss is an invariant violation surfaced loudly
            // (debug-assert + log) and degrades recoverably: the coordinate
            // recording is skipped rather than fabricating a duplicate
            // inode.
            if let Ok(parent) = self.publication_parent(parent_facts) {
                positive.inode.record_copyup_transition(parent, name);
            }
        }
        self.publish_binding(&parent_id, name, binding.clone());
        Ok(LookupOutcome {
            binding,
            is_stale_upper,
        })
    }

    /// Creates or reuses the shared [`OverlayInode`] for `facts`.
    ///
    /// The positive branch only: the visible-metadata source keys the
    /// inode-cache entry (`RealObjectKey::from_facts`), and the published
    /// `object_id` is precomputed from `IdentityPolicy` BEFORE the inode-cache
    /// check-and-create: the upper-source lower-id read (`read_lower_id`) may
    /// block on the underlying xattr and must never run inside the cache's
    /// upgraded guard, whose create closure allocates only. On
    /// `read_lower_id` error the visible-source projection is used (logged;
    /// never silently wrong).
    ///
    /// # Mount reference
    ///
    /// The canonical `Weak<OverlayFs>` stamped on every created inode comes
    /// from `OverlayFs::self_weak` — established by `Arc::new_cyclic` in
    /// `OverlayFs::new` (ramfs precedent) — NOT by downcasting the root
    /// inode. The weak upgrades exactly while the mount lives, so every
    /// created inode's `fs()` upgrade succeeds while the mount lives.
    fn project_inode(&self, facts: &OverlayObjectFacts) -> Arc<OverlayInode> {
        let source = visible_source(facts);
        let key = RealObjectKey::from_facts(facts);
        let is_directory =
            facts.kind == PositiveKind::Merged || source.real_inode().type_().is_directory();
        // The visible-source projection is the single conservative fallback
        // of this block — bound once and reused by every arm: the expression
        // `self.identity().project_object_id(source, is_directory)` is
        // written exactly once (in the closure) and the closure is invoked
        // by all five uses below (flattened — no nested match). The `_fn`
        // suffix marks the binding as callable at every use site.
        let fallback_fn = || self.identity().project_object_id(source, is_directory);
        let object_id = if source.layer_index() == 0 {
            match self.read_lower_id(source.real_inode()) {
                // Defensive: the record was device-validated at the read boundary, so
                // `None` here is the absent/ambiguous-device corner; the
                // visible-source projection is the conservative fallback.
                Ok(Some(record)) => {
                    // The record is accepted only when its real inode is
                    // consistent with the retained same-layer lower of the
                    // fresh facts; otherwise the visible-source projection
                    // is the conservative fallback.
                    if self.origin_real_ino_resolves(&record, facts) {
                        self.identity()
                            .project_object_id_from_lower_id(&record, is_directory)
                            .unwrap_or_else(fallback_fn)
                    } else {
                        fallback_fn()
                    }
                }
                Ok(None) => fallback_fn(),
                Err(err) => {
                    warn!(
                        "failed to read the lower-id record of the upper source; \
                         falling back to the visible-source projection: {:?}",
                        err
                    );
                    fallback_fn()
                }
            }
        } else {
            fallback_fn()
        };
        // Clone the visible source before the closures move `facts`: the
        // get-or-create predicate validates a cached hit against this real
        // inode, replacing an ino-reuse stale occupant. The fresh-truth
        // upper presence is captured here as well, because the predicate
        // must distinguish a lower-only fresh truth (below) from an
        // upper-backed one.
        let source_inode = visible_source(facts).real_inode().clone();
        let fresh_is_lower_only = facts.upper().is_none();
        let fs = self.self_weak.clone();
        let facts = facts.clone();
        self.inodes().get_or_create(
            key,
            move |carrier| {
                if fresh_is_lower_only {
                    // The fresh truth is lower-only: reuse only an inode
                    // whose visible source is exactly this lower — the
                    // in-flight lower-only projection. A stale-upper inode
                    // (visible source = the vanished upper, this lower merely
                    // retained in its snapshot) must NOT be reused even
                    // though `contains_real_inode` would match the retained
                    // lower; serving its dead-upper metadata would be wrong.
                    // An upper-still-alive in-flight projection is unaffected:
                    // its fresh truth has an upper and takes the
                    // `contains_real_inode` branch below.
                    Arc::ptr_eq(
                        visible_source(&carrier.facts_snapshot()).real_inode(),
                        &source_inode,
                    )
                } else {
                    carrier.facts_snapshot().contains_real_inode(&source_inode)
                }
            },
            move || {
                Arc::new(OverlayInode {
                    fs,
                    key: Mutex::new(key),
                    facts: Mutex::new(facts),
                    dir_transaction_lock: if is_directory {
                        Some(Mutex::new(()))
                    } else {
                        None
                    },
                    object_id,
                    extension: Extension::new(),
                    // The readdir index is `Some` iff this object is a directory
                    // (empty initial index); the copy-up transition coordinate
                    // starts `None` (copy-up records the first positive-binding
                    // publication).
                    readdir_index: if is_directory {
                        Some(Mutex::new(ReaddirIndex::new()))
                    } else {
                        None
                    },
                    copyup_transition: Mutex::new(None),
                })
            },
        )
    }

    /// Creates or reuses the shared [`OverlayInode`] for a freshly created
    /// upper object.
    ///
    /// The namespace-mutation recipes call this inline for the new upper
    /// object before `BindingCache::insert`. It reuses the exact
    /// `project_inode` semantics — inode-cache `get_or_create` by
    /// `RealObjectKey`, facts/object_id initialization — so a second
    /// projection of the same upper real object reuses the same identity.
    pub(in crate::fs::fs_impls::overlayfs) fn project_new_upper(
        &self,
        facts: &OverlayObjectFacts,
    ) -> Arc<OverlayInode> {
        self.project_inode(facts)
    }

    /// Publishes `binding` for `(parent_id, name)` into the binding cache.
    ///
    /// Called by `lookup_binding` under the parent `DIR` transaction — and by
    /// the namespace-mutation recipes' shared publication entry point
    /// (`dir/create.rs::publish_positive_binding`) — so the
    /// check-act-publish sequence stays atomic per directory; the entry is an
    /// immutable `Arc<Binding>` snapshot (replaced, never mutated in place).
    pub(in crate::fs::fs_impls::overlayfs) fn publish_binding(
        &self,
        parent_id: &RealObjectKey,
        name: &str,
        binding: Binding,
    ) {
        let key = BindingKey {
            parent_id: *parent_id,
            name: name.into(),
        };
        self.bindings().insert(key, Arc::new(binding));
    }

    /// Returns the published `Arc<OverlayInode>` of the parent directory
    /// whose facts are `parent_facts` — the lookup receiver (copy-up hook
    /// support).
    ///
    /// Every live inode — the root (registered by `OverlayInode::new_root`
    /// alongside the mount slot), every non-root inode (`project_inode`),
    /// and every copied-up parent ([`OverlayInode::replace_facts`], which
    /// aliases the registration so the old and new visible-source keys both
    /// resolve to the one inode) — is registered in `InodeCache` under its
    /// *current* visible-source key.
    /// The read-only probe therefore returns the actual lookup parent — the
    /// one inode registered for the visible-source key — with exactly one
    /// inode per key on every path; no key-equality root check exists and
    /// no duplicate is ever projected.
    ///
    /// A probe miss violates the registration invariant (a live parent is
    /// never missing from the cache): the miss is surfaced with a
    /// `debug_assert!` and an error log and returns `Err` so the hook caller
    /// degrades recoverably (skipping the coordinate recording) instead of
    /// silently minting a second inode. The probe holds no
    /// upgradeable-reader slot, so it never re-enters the inode cache's
    /// single upgradeable slot.
    fn publication_parent(&self, parent_facts: &OverlayObjectFacts) -> Result<Arc<OverlayInode>> {
        let key = RealObjectKey::from_facts(parent_facts);
        match self.inodes().get(key) {
            Some(parent) => Ok(parent),
            None => {
                debug_assert!(
                    false,
                    "a live overlay parent is always registered under its current visible-source key"
                );
                error!(
                    "overlay parent identity inconsistency: no inode-cache entry for \
                     visible-source key {:?}",
                    key
                );
                Err(Error::with_message(
                    Errno::EIO,
                    "the overlay parent inode is not registered under its visible-source key",
                ))
            }
        }
    }
}

/// Returns the visible-metadata source of `facts`: the upper real object when
/// present, else the topmost lower (`lowers[0]`).
///
/// The identical selection runs on the parent facts (binding-key derivation
/// in `lookup_binding`), the child facts (key, directory and identity
/// projection in `project_inode`), and every `OverlayInode` metadata method
/// in inode.rs (size/metadata/type_/mode/owner/group/times/root), so the
/// `lowers[0]` indexing invariant (`upper.is_some() || !lowers.is_empty()`) is
/// documented and enforced in exactly one place. The key derivation is the
/// second half of that rule and is centralized in `RealObjectKey::from_facts`.
pub(super) fn visible_source(facts: &OverlayObjectFacts) -> &RealObject {
    match &facts.upper {
        Some(upper) => upper,
        None => &facts.lowers[0],
    }
}
