// SPDX-License-Identifier: MPL-2.0

//! The overlayfs projection and identity subsystem.
//!
//! # Concepts
//!
//! A **projection** is the deterministic mapping from a name's real
//! (underlying) layer object — or, for a merged directory, its upper real
//! object together with its lower stack — to the overlay's visible identity
//! for it: the object kind, the projected dev/ino, and the reusable
//! [`OverlayInode`] — instead of a copy of the real object.
//! A **binding** is the remembered result of one `(parent, name)` lookup:
//! `Positive` (a pinned [`OverlayInode`]) or
//! `Negative` (why the name is hidden or absent).
//!
//! This module owns the lookup path: it resolves a name upper-first across
//! the layer stack, projects the winning real object, and publishes the
//! binding in the mount-wide [`BindingCache`].
//!
//! # Structure
//!
//! | Submodule | Owns |
//! |---|---|
//! | `binding_cache` | The `Binding` type and the mount-wide binding cache. |
//! | `lookup` | Real-object projection and the upper-first layer lookup core. |
//! | `identity` | Dev/ino identity projection. |
//! | `inode_cache` | Inode identity-reuse cache. |
//! | `lower_id` | The durable lower-source identity record. |
//!
//! The core inode carrier [`OverlayInode`] lives in the root `inode` module.
//!
//! # References
//!
//! - Overlayfs (Linux overlay filesystem):
//!   <https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html>

mod binding_cache;
mod identity;
mod inode_cache;
mod lookup;
mod lower_id;

pub(super) use binding_cache::{
    Binding, BindingCache, BindingKey, HiddenEvidence, NegativeBinding, PositiveBinding,
    PositiveKind,
};
pub(super) use identity::{IdentityPolicy, LowerLayerIdentity, ObjectId};
pub(super) use inode_cache::{InodeCache, RealObjectKey};
use lookup::LayerLookup;
pub(super) use lookup::{RealObject, is_whiteout_inode};

use crate::{
    fs::{
        fs_impls::overlayfs::{
            inode::{ObjectFacts, OverlayInode},
            readdir_index::ReaddirIndex,
            superblock::OverlayFs,
        },
        vfs::inode::Extension,
    },
    prelude::*,
};

/// The result of one `(parent_id, name)` lookup.
///
/// `is_stale_upper` is true when the fresh layer truth no longer contains the
/// upper entry of a previously published upper-backed positive binding and no
/// whiteout covers the name. Most consumers ignore the signal; the remove
/// path consumes it to surface `ESTALE` instead of re-exposing the lower
/// counterpart.
pub(super) struct LookupOutcome {
    /// A verified cached binding, or the freshly rebuilt binding from the
    /// layer truth.
    pub(super) binding: Binding,
    /// Whether this lookup observed the stale-upper class.
    pub(super) is_stale_upper: bool,
}

impl OverlayFs {
    /// Resolves one `name` under `parent_facts` into a [`LookupOutcome`].
    ///
    /// The flow is verify-then-serve: the layer-ordered lookup re-observes
    /// the fresh layer truth, and a cached binding is served only when it
    /// matches that truth; otherwise the binding is rebuilt and published.
    pub(super) fn lookup_binding(
        &self,
        parent_facts: &ObjectFacts,
        name: &str,
    ) -> Result<LookupOutcome> {
        let parent_id = RealObjectKey::from_facts(parent_facts);
        let truth = self.lookup_in_layers(parent_facts, name)?;
        let is_stale_upper = if let Some(binding) = self.bindings.get(&parent_id, name) {
            if binding.matches_truth(&truth) {
                return Ok(LookupOutcome {
                    binding: binding.as_ref().clone(),
                    is_stale_upper: false,
                });
            }
            binding.is_stale_upper(&truth)
        } else {
            false
        };
        let binding = match truth {
            LayerLookup::Positive(facts) => {
                let inode = self.project_inode(&facts);
                Binding::Positive(PositiveBinding::new(inode))
            }
            LayerLookup::Negative(negative) => Binding::Negative(negative),
        };
        // Record the copy-up transition coordinate — the `(parent, name)`
        // under which this inode first appeared on the upper. The
        // per-inode guard keeps the first positive binding's coordinate;
        // later lookups leave it unchanged.
        if let Binding::Positive(positive) = &binding {
            let parent_key = RealObjectKey::from_facts(parent_facts);
            if let Some(parent) = self.inodes.get(parent_key) {
                positive.inode.try_record_copyup_transition(parent, name);
            } else {
                debug_assert!(
                    false,
                    "a live overlay parent is always registered under its current visible-source key"
                );
                error!(
                    "overlay parent identity inconsistency: no inode-cache entry for \
                     visible-source key {:?}",
                    parent_key
                );
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
    /// The `object_id` is precomputed from `IdentityPolicy` before the
    /// inode-cache check-and-create, because the upper-source lower-id read
    /// may block on the underlying xattr and must never run inside the
    /// cache's upgraded guard.
    pub(super) fn project_inode(&self, facts: &ObjectFacts) -> Arc<OverlayInode> {
        let source = facts.visible_source();
        let key = RealObjectKey::from_facts(facts);
        let is_directory =
            facts.kind == PositiveKind::Merged || source.real_inode().type_().is_directory();
        let fallback_fn = || self.identity.project_object_id(source, is_directory);
        let object_id = if source.layer_index() == 0 {
            match self.read_lower_id(source.real_inode()) {
                // Defensive: the record was device-validated at the read boundary,
                // so `None` here is the absent/ambiguous-device corner.
                Ok(Some(record)) => {
                    // The record is accepted only when its real inode is
                    // consistent with the retained same-layer lower of the
                    // fresh facts.
                    if self.identity.origin_real_ino_resolves(&record, facts) {
                        self.identity
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
        let source_inode = facts.visible_source().real_inode().clone();
        let fresh_is_lower_only = facts.upper.is_none();
        let fs = self.self_weak.clone();
        let facts = facts.clone();
        self.inodes.get_or_create(
            key,
            move |carrier| {
                if fresh_is_lower_only {
                    // Reuse only an inode whose visible source is exactly
                    // this lower; a stale-upper inode must not be reused even
                    // though `contains_real_inode` matches the retained
                    // lower, because its dead-upper metadata would be wrong.
                    Arc::ptr_eq(
                        carrier.facts_snapshot().visible_source().real_inode(),
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

    /// Publishes `binding` for `(parent_id, name)` into the binding cache.
    pub(super) fn publish_binding(&self, parent_id: &RealObjectKey, name: &str, binding: Binding) {
        let key = BindingKey {
            parent_id: *parent_id,
            name: name.into(),
        };
        self.bindings.insert(key, Arc::new(binding));
    }
}
