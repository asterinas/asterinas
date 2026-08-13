// SPDX-License-Identifier: MPL-2.0

//! The copy-up coordination (`CUL`) payload.
//!
//! This module owns the two types of the `copyup/coordination.rs` surface:
//! [`CopyUpTransition`] — the durable per-object publication coordinate
//! (`publication_parent` + `name` + `phase`) — and its [`CopyUpPhase`]
//! transition marker. The payload is exactly the promotion coordinate plus
//! phase; no unrelated fields and no stored "copy-up completed" history
//! marker (the upper authority in the facts record is the durable outcome).
//!
//! Stored under
//! `OverlayInode::copyup_transition: Mutex<Option<CopyUpTransition>>`, the
//! `CUL` domain (field in `projection/inode.rs`). `None` only before
//! the first positive-binding publication; the guard is a sleep-capable
//! `ostd::sync::Mutex` (promotion can BIO under it), and waiters hold nothing
//! while blocked on `lock()`. The coordinate is recorded once at the first
//! positive-binding publication and read by every winner; `phase` transitions
//! advance the coordinate's marker, never the authority. The only consumers
//! are `record_copyup_transition` (`copyup/mod.rs`, writes the coordinate),
//! `ensure_upper_authority` (`copyup/trigger.rs`, reads it under `CUL`), and
//! `promote` (`copyup/promote.rs`, advances the phase).

use crate::{fs::fs_impls::overlayfs::projection::OverlayInode, prelude::*};

/// The copy-up publication coordinate and phase of one logical overlay
/// object.
///
/// The `CUL`-domain payload stored at
/// `OverlayInode::copyup_transition`: the promotion coordinate plus phase,
/// recorded exactly once at the first positive-binding publication and read
/// by every subsequent winner (the coordinate is immutable after the first
/// record; only `phase` transitions). The strong [`Arc`] pin in
/// [`Self::publication_parent`] forms the publication-parent chain (acyclic,
/// root-terminated; no cycle), so the trigger's top-down ancestor walk
/// terminates at the upper-backed root and never re-enters the same instance.
pub(in crate::fs::fs_impls::overlayfs) struct CopyUpTransition {
    /// The logical parent overlay inode (may still be lower-backed; the
    /// parent's upper existence is resolved by the trigger's ancestor walk,
    /// never assumed ready).
    pub(super) publication_parent: Arc<OverlayInode>,
    /// The exact publication name under `publication_parent`; non-empty.
    pub(super) name: String,
    /// The transition marker consumed by the next winner entry; the upper
    /// authority in the facts record, not this marker, is the durable outcome.
    pub(super) phase: CopyUpPhase,
}

/// The transition marker of one copy-up coordination.
///
/// Semantic mapping: lower-authoritative = `facts.upper` none +
/// [`CopyUpPhase::Idle`]; promotion-in-progress = the `CUL` guard held by the
/// winner (observable only as mutex contention); upper-authoritative =
/// `facts.upper` some; retryable failure = the error returned to the caller
/// (authority unchanged, no durable marker needed); reconcile-required =
/// [`CopyUpPhase::ReconcilePending`]. No "copy-up completed" history marker
/// is stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum CopyUpPhase {
    /// No unfinished transition; a lower authority (if any) is clean.
    Idle,
    /// Physical publication happened but semantic publication failed; the
    /// upper object at `(publication_parent, name)` must be verified before
    /// reuse.
    ReconcilePending,
}
