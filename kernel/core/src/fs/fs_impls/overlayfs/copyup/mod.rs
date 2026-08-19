// SPDX-License-Identifier: MPL-2.0

//! The module root of the copy-up authority.
//!
//! This module declares the three `copyup/*` submodules and hosts the thin
//! inode-level delegation entry `try_record_copyup_transition`.
//! The shared delegation selectors and the VFS helper bodies live in the
//! `inode` module; the winner/waiter trigger and the promotion body live in
//! the two sibling submodules.
//!
//! ## Per-call delegation
//!
//! Every call re-resolves the current authority; there is no per-open
//! real-inode view object to reuse across calls.
//!
//! ## References
//!
//! - Linux `ovl_real_file_path` follow-copy-up:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/file.c#L128-L171>

use self::coordination::{CopyUpPhase, CopyUpTransition};
use crate::{fs::fs_impls::overlayfs::inode::OverlayInode, prelude::*};

pub(super) mod coordination;

pub(super) mod promote;
mod trigger;

impl OverlayInode {
    /// Records the copy-up transition coordinate at the first positive
    /// binding publication.
    ///
    /// The coordinate is set once — the first positive binding wins; the
    /// non-blocking `try_lock` skips when contended because a transition
    /// already running has already set it.
    pub(super) fn try_record_copyup_transition(
        &self,
        publication_parent: Arc<OverlayInode>,
        name: &str,
    ) {
        let Some(mut guard) = self.try_lock_copyup_transition() else {
            return;
        };
        if guard.is_some() {
            return;
        }
        *guard = Some(CopyUpTransition {
            publication_parent,
            name: String::from(name),
            phase: CopyUpPhase::Idle,
        });
    }
}
