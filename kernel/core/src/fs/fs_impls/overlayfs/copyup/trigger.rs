// SPDX-License-Identifier: MPL-2.0

//! The copy-up promotion trigger — the single winner/waiter entry that gives
//! an existing logical overlay object upper authority.
//!
//! [`OverlayInode::ensure_upper_authority`] is the only entry into
//! lower-to-upper authority promotion. Mutating entries (write-intent `open`,
//! `resize`, `fallocate`, and the metadata-security `setattr` path) derive a
//! coarse mutating-vs-read-only class from the VFS surface, pass the EROFS
//! gate upstream, and call this method; read-only operations never reach it.
//! It takes **no trusted intent parameter**: the promotion scope — how many
//! ancestor parents to promote and which object-kind recipe to run — is
//! decided inside the per-object `CUL` domain by inspection of the facts, the
//! per-object publication coordinate (`copyup_transition`), and the parent
//! chain.
//!
//! Winner/waiter flow: the top-down ancestor walk promotes the parent chain
//! strictly parent-`CUL`-before-child-`CUL`, terminating at the upper-backed
//! root. After the walk, the winner's arbitration guard
//! (`copyup_transition`) is acquired and HELD through publication: under the
//! guard the winner re-snapshots the facts (waiter leg), re-reads the
//! coordinate once, and runs the promotion body ([`OverlayInode::promote`],
//! promote.rs) with the coordinate carried as parameters. `promote`'s
//! helpers consume the passed coordinate and never re-acquire
//! `copyup_transition`, so the non-reentrant `ostd::sync::Mutex` is never
//! re-entered and no concurrent winner can interleave between the
//! re-snapshot and the semantic publication — the double copy-up TOCTOU is
//! closed. Waiters block on the sleep-capable `CUL` lock and re-observe
//! authority immediately after acquisition (fast-path re-snapshot), never
//! holding `CUL`/`INODE`/`UPPER` while sleeping. The ReconcilePending marker
//! (recovery) is derived from the coordinate phase inside `promote` under the
//! held guard — no redundant bool crosses this boundary. On return every
//! Overlay lock is released; `Ok(())` is the sole success return value and the
//! caller re-observes authority via `facts_snapshot`.

/// The maximum depth of the copy-up ancestor recursion.
///
/// `ensure_upper_authority_inner` recurses once per consecutive lower-backed
/// publication parent; each frame keeps only two live `Arc`s (`_fs` and
/// `publication_parent`) and releases every guard before the next recursion,
/// so a single frame is ~96-128 B typically and at most ~256 B pessimistically.
/// 1024 × 256 B = 256 KiB ≤ half of the default 512 KiB kernel task stack
/// (128 pages × 4 KiB; `OSTD_TASK_STACK_SIZE_IN_PAGES` may override), so a
/// chain deeper than this limit fails closed with `ELOOP` instead of ever
/// risking a kernel-stack overflow.
const MAX_COPYUP_DEPTH: usize = 1024;

use crate::{fs::fs_impls::overlayfs::projection::OverlayInode, prelude::*};

impl OverlayInode {
    /// Promotes this logical object to upper authority, winning or waiting on
    /// the per-object `CUL` (`copyup_transition`).
    ///
    /// Returns `Ok(())` when the object is already upper-backed (idempotent
    /// fast path), when it became upper-backed while waiting for the `CUL`
    /// (waiter leg), or after this task won and completed the promotion;
    /// returns `Err(Errno::ENOENT)` when no publication coordinate is
    /// recorded (defensive guard), and propagates any underlying recipe
    /// failure unchanged.
    ///
    /// The top-down ancestor walk recurses at most [`MAX_COPYUP_DEPTH`] levels
    /// (see the constant's stack-budget note); a deeper consecutive
    /// lower-backed ancestor chain fails closed with `Errno::ELOOP` instead of
    /// risking kernel-stack overflow, with no change to any success path.
    ///
    /// Lock contract: the brief `CUL` read that captures `publication_parent`
    /// releases its guard before the recursive ancestor walk, so the parent
    /// `CUL` is always acquired strictly before the child `CUL`; the
    /// arbitration guard is then acquired and held THROUGH the winner body —
    /// the re-snapshot, the coordinate re-read, and
    /// [`OverlayInode::promote`] (which carries the coordinate and never
    /// re-acquires the guard; `ostd::sync::Mutex` is non-reentrant). No
    /// Overlay lock crosses the return boundary.
    pub(in crate::fs::fs_impls::overlayfs) fn ensure_upper_authority(&self) -> Result<()> {
        self.ensure_upper_authority_inner(0)
    }

    /// The recursive body of [`OverlayInode::ensure_upper_authority`]; `depth`
    /// is the number of ancestor recursions already performed (0 at the entry,
    /// incremented once per consecutive lower-backed publication parent).
    fn ensure_upper_authority_inner(&self, depth: usize) -> Result<()> {
        // Step 1 — mount-lifetime pin: the `Weak<OverlayFs>` upgrade proves
        // the mount is alive and pins it for the trigger's duration (no
        // `.unwrap()`/`.expect()`).
        let _fs = self.fs_arc()?;

        // Step 2 — idempotent upper fast path: facts inspection only, no
        // `CUL`, no second temporary, no second transfer.
        if self.facts_snapshot().upper().is_some() {
            return Ok(());
        }

        // Step 3 — the publication coordinate (defensive guard): the brief
        // `CUL` read clones the logical parent out of the coordinate so the
        // guard is released before the recursive ancestor walk (`Some` after
        // the first positive-binding publication).
        let publication_parent = {
            let transition = self.copyup_transition.lock();
            let Some(coordinate) = transition.as_ref() else {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the overlay object has no recorded copy-up publication coordinate",
                ));
            };
            coordinate.publication_parent.clone()
        };

        // Step 4 — top-down ancestor walk: the parent promotes its own
        // ancestors first, so the parent `CUL` is strictly acquired before
        // the child `CUL`; the recursion terminates at the upper-backed root
        // and never re-enters the same instance (acyclic chain). The depth
        // counter bounds the chain: more than `MAX_COPYUP_DEPTH` consecutive
        // lower-backed ancestors fails closed with `ELOOP` (never a stack
        // overflow); the success-path behavior is unchanged.
        if depth >= MAX_COPYUP_DEPTH {
            return_errno_with_message!(
                Errno::ELOOP,
                "the copy-up ancestor chain exceeds the depth limit"
            );
        }
        publication_parent.ensure_upper_authority_inner(depth + 1)?;

        // Step 5 — winner/waiter serialization: the sleep-capable `CUL` wait.
        // Waiters hold nothing while blocked on `lock()`; the guard is then
        // held for the arbitration, the re-snapshot, and the whole winner
        // body (promote runs under the guard, so no second winner can
        // interleave between the re-snapshot and the semantic publication).
        let mut transition = self.copyup_transition.lock();

        // Step 6 — re-snapshot under the guard: another task won and promoted
        // while this task waited; re-observe upper authority and return the
        // same `Ok(())` success value (waiter path).
        if self.facts_snapshot().upper().is_some() {
            return Ok(());
        }

        // Step 7 — winner body under the held guard: read the coordinate once
        // and run `promote`, which verifies a pending reconcile and runs the
        // object-kind recipe (file/symlink/dir/special) through publication.
        // The phase transitions (ReconcilePending on reconcile, Idle on
        // success) are written through the same coordinate borrow; promote's
        // helpers take the passed coordinate and never re-read
        // `copyup_transition` (no non-reentrant deadlock).
        let coordinate = match transition.as_mut() {
            Some(coordinate) => coordinate,
            None => {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the overlay object has no recorded copy-up publication coordinate",
                ));
            }
        };
        let publication_parent = coordinate.publication_parent.clone();
        let name = coordinate.name.clone();

        // Step 8 — the winner body; Step 9 (release `CUL`) is the guard drop
        // at this function's return. The ReconcilePending marker is derived
        // inside `promote` from the passed coordinate's phase under the held
        // guard (no redundant bool crosses the trigger boundary).
        self.promote(&publication_parent, &name, coordinate)?;
        Ok(())
    }
}
