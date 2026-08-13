// SPDX-License-Identifier: MPL-2.0

//! The link recipe.
//!
//! This module owns the two recipe helpers on [`OverlayInode`] —
//! [`OverlayInode::link_source`] (source authority promotion to the shared
//! upper real inode) and [`OverlayInode::link_over_whiteout`] (workdir
//! hard-link staging plus atomic rename-over replacement of a published
//! whiteout target). The `Inode::link` entry itself lives in the sibling
//! `dir/mod.rs` (the thin mutation entries), which composes these helpers
//! with the target publication sequence (`BindingCache::insert` +
//! `readdir_index_insert`) under the target parent `DIR`.
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! Lock contract: neither helper acquires or holds any Overlay lock. They run
//! inside the caller's target-parent `DIR` domain established by
//! `lock_dir_transaction` in `dir/mod.rs`; the source promotion
//! (`ensure_upper_authority`) acquires the per-object `CUL` and `INODE`
//! domains in the `DIR -> CUL -> INODE` order (released on publication or
//! return); the underlying upper/workdir operations (`link`/`rename`) may
//! block and run in the sleep-capable domain, never under `WL` or any spin
//! lock. The workdir temp is private staging and is cleaned best-effort on
//! the rename-over failure path — an explicit fallible operation, never an
//! RAII-durable-rollback.
//!
//! Degradation note: without a persistent origin index, two lower aliases of
//! one lower inode that copy up separately may become two distinct upper
//! inodes; upper-authoritative sources always share one upper inode (the real
//! hard link published here). The no-index degradation is an acknowledged
//! limitation, never papered over.

use crate::{
    fs::{
        fs_impls::overlayfs::{copyup::WorkdirTempRequest, projection::OverlayInode},
        vfs::{inode::RenameMode, path::Path},
    },
    prelude::*,
};

impl OverlayInode {
    /// Promotes the link source to upper authority and resolves the shared
    /// upper real object's dentry-anchored path.
    ///
    /// The source branch of the link recipe: `old.ensure_upper_authority()`
    /// makes the source upper-authoritative (idempotent fast path when
    /// already upper-backed), then the facts' upper real object resolves to
    /// its dentry-anchored [`Path`] — the single upper real object's path
    /// that the new target hard link shares with the source. The caller (the
    /// `dir/mod.rs` `Inode::link` entry) composes this per-branch promotion
    /// with the target-parent promotion in stable object-identity order; this
    /// helper covers the source branch only.
    ///
    /// Lock contract: runs under the caller's target-parent `DIR`; the
    /// promotion acquires `CUL` → `INODE` in order and releases them on
    /// publication or return. No Overlay lock is acquired or held by this
    /// method itself and none crosses the return boundary.
    ///
    /// Returns the shared upper real object's dentry-anchored `Path` on
    /// success; propagates any
    /// promotion error unchanged (`Err(Errno::ENOENT)` on the defensive guard
    /// when no copy-up coordinate is recorded, and any underlying recipe
    /// failure).
    pub(super) fn link_source(&self, old: &Arc<OverlayInode>) -> Result<Path> {
        old.ensure_upper_authority()?;
        let facts = old.facts_snapshot();
        let upper = facts.upper().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the link source has no upper real object after promotion",
            )
        })?;
        upper.real_path()
    }

    /// Replaces a published whiteout target with a hard link to the shared
    /// source upper real object.
    ///
    /// The target-whiteout leg of the link recipe (Linux
    /// `ovl_create_over_whiteout` hardlink leg): the shared source upper
    /// object's dentry-anchored path is staged as a private workdir hard link
    /// under a unique temp name (`generate_workdir_temp_name`), then
    /// atomically renamed over the whiteout at `name` in the target upper
    /// parent with
    /// `RenameMode::Replace`. The whiteout is consumed by the replacement and
    /// never re-cached; the staged hard link becomes the visible upper object
    /// at the target name.
    ///
    /// Workdir temporaries stay private staging: the temp is never a
    /// lookup/readdir/`ReaddirIndex` source. On a rename-over failure the
    /// staged hard link is removed best-effort via
    /// `cleanup_workdir_temp`; a cleanup failure is a known workdir-cleanup
    /// debt and never becomes a visible namespace entry.
    ///
    /// Lock contract: runs under the caller's target-parent `DIR`; the
    /// underlying upper operations (`Path::link`/`Path::rename`) may block
    /// and run in the sleep-capable domain, never under `WL` or any spin
    /// lock. The workdir staging workspace resolves through the single shared
    /// resolver `OverlayInode::workdir_root_path` (no workdir side effect
    /// without a writable claim). No Overlay lock is acquired or held by this
    /// method and none crosses the return boundary.
    pub(super) fn link_over_whiteout(&self, name: &str, source_path: &Path) -> Result<()> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let temp = fs.create_workdir_temp(
            name,
            &upper_parent_path,
            WorkdirTempRequest::Link {
                source: source_path.clone(),
            },
        )?;
        let workdir_path = self.workdir_root_path()?;
        // Step 1 — the hard-link leg: stage the shared source upper real
        // inode as a private workdir hard link under the unique temp name.
        // Step 2 — atomic rename-over: replace the published whiteout at
        // `name` with the staged hard link (`Replace`; the whiteout is
        // consumed, never re-cached). On failure the staged temp is removed
        // best-effort so no workdir residue outlives the recipe.
        if let Err(err) =
            workdir_path.rename(temp.name(), &upper_parent_path, name, RenameMode::Replace)
        {
            let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
            return Err(err);
        }
        Ok(())
    }
}
