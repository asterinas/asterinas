// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The copy-up promotion trigger — the single winner/waiter entry that
//! promotes an existing logical overlay object to upper authority.
//!
//! [`OverlayInode::ensure_upper_authority`] is the only entry; promotion runs
//! inside the per-object copy-up coordination lock. The ancestor walk
//! promotes the parent object before the child; the winner holds the guard
//! through publication while the helpers never re-acquire `copyup_transition`,
//! closing the double copy-up TOCTOU.

/// The maximum depth of the copy-up ancestor recursion.
///
/// Each frame keeps only two live `Arc`s and no guard, so 1024 frames fit
/// within the default kernel task stack; a deeper chain fails closed with
/// `ELOOP` instead of risking a stack overflow.
const MAX_COPYUP_DEPTH: usize = 1024;

use crate::{fs::fs_impls::overlayfs::inode::OverlayInode, prelude::*};

impl OverlayInode {
    /// Promotes this logical object to upper authority, winning or waiting on
    /// the per-object copy-up coordination guard (`copyup_transition`).
    ///
    /// Returns `Ok(())` once the object is upper-backed (idempotent fast
    /// path, waiter leg, or this task's own completed promotion), `Err` when
    /// no publication coordinate is recorded, and propagates any underlying
    /// recipe failure unchanged. A deeper ancestor chain than
    /// [`MAX_COPYUP_DEPTH`] fails closed with `Errno::ELOOP`.
    pub(in overlayfs) fn ensure_upper_authority(&self) -> Result<()> {
        self.ensure_upper_authority_inner(0)
    }

    /// The recursive body of [`OverlayInode::ensure_upper_authority`]; `depth`
    /// is the number of ancestor recursions already performed (0 at the entry,
    /// incremented once per consecutive lower-backed publication parent).
    fn ensure_upper_authority_inner(&self, depth: usize) -> Result<()> {
        // Pins the owning mount for the trigger's duration.
        let _fs = self.fs_arc()?;

        if self.facts_snapshot().upper.is_some() {
            return Ok(());
        }

        // Publication coordinate: the brief
        // `copyup_transition` read clones the logical parent and name so the
        // guard is released before the recursive ancestor walk; both are
        // fixed once the coordinate is recorded, so the winner body reuses
        // this single binding.
        let (publication_parent, name) = {
            let transition = self.lock_copyup_transition();
            let Some(coordinate) = transition.as_ref() else {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the overlay object has no recorded copy-up publication coordinate",
                ));
            };
            (
                coordinate.publication_parent.clone(),
                coordinate.name.clone(),
            )
        };

        if depth >= MAX_COPYUP_DEPTH {
            return_errno_with_message!(
                Errno::ELOOP,
                "the copy-up ancestor chain exceeds the depth limit"
            );
        }
        publication_parent.ensure_upper_authority_inner(depth + 1)?;

        // Winner/waiter serialization: the sleep-capable
        // `copyup_transition` lock wait.
        let mut transition = self.lock_copyup_transition();

        // Re-snapshot under the guard: another task won and promoted while
        // this task waited; re-observe upper authority and return the same
        // `Ok(())` success value (waiter path).
        if self.facts_snapshot().upper.is_some() {
            return Ok(());
        }

        // Winner body:
        let coordinate = match transition.as_mut() {
            Some(coordinate) => coordinate,
            None => {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the overlay object has no recorded copy-up publication coordinate",
                ));
            }
        };
        self.promote(&publication_parent, &name, coordinate)?;
        Ok(())
    }
}
