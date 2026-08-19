// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The per-object copy-up coordination payload.
//!
//! This module owns the two types of the coordination surface:
//! [`CopyUpTransition`] — the durable per-object publication coordinate
//! (`publication_parent` + `name` + `phase`) — and its [`CopyUpPhase`]
//! transition marker.
//!
//! The guard is a sleep-capable
//! `ostd::sync::Mutex` because promotion can perform block I/O under
//! it.

use crate::{fs::fs_impls::overlayfs::inode::OverlayInode, prelude::*};

/// The copy-up publication coordinate and phase of one logical overlay object.
///
/// Recorded exactly once at the first positive-binding publication; its
/// coordinate fields are fixed and only `phase` can transition thereafter;
/// the upper authority in the facts record is the durable outcome, so no
/// copy-up-completed history marker exists. The publication-parent chain is
/// acyclic and root-terminated, so the trigger's top-down ancestor walk
/// terminates and never re-enters the same instance.
pub(in overlayfs) struct CopyUpTransition {
    /// The logical parent overlay inode; its upper existence is resolved by
    /// the trigger's ancestor walk, which checks the parent's upper existence
    /// and may promote it first.
    pub(super) publication_parent: Arc<OverlayInode>,
    pub(super) name: String,
    pub(super) phase: CopyUpPhase,
}

/// The transition marker of one copy-up coordination.
///
/// Maps the copy-up phase values to their semantic states:
/// - lower-authoritative: `facts.upper` is `None` and [`CopyUpPhase::Idle`].
/// - promotion-in-progress: the `copyup_transition` guard is held by
///   the winner (observable only as mutex contention).
/// - upper-authoritative: `facts.upper` is `Some`.
/// - retryable failure: the error is returned to the caller (authority
///   unchanged, no durable marker needed).
/// - reconcile-required: [`CopyUpPhase::ReconcilePending`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CopyUpPhase {
    /// The coordinate carries no unfinished transition; a lower authority (if
    /// any) is clean.
    Idle,
    /// Physical publication happened but semantic publication failed; the
    /// upper object at `(publication_parent, name)` must be verified before
    /// reuse.
    ReconcilePending,
}
