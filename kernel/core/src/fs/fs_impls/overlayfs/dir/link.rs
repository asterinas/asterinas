// SPDX-License-Identifier: MPL-2.0

//! The link recipe.
//!
//! Lock contract: this module enters the per-object copy-up coordination
//! lock only via source promotion.
//!
//! Owns [`OverlayInode::link_source`] and [`OverlayInode::link_over_whiteout`];
//! the `Inode::link` entry composes them under that transaction lock;
//! temp cleanup on failure is explicit and fallible, never an RAII rollback.
//!
//! Degradation note: without a persistent origin index (a lower-origin
//! identity map used to deduplicate copy-up targets), two lower aliases of
//! one lower inode that copy up separately may become two distinct upper
//! inodes. This split is accepted: each copy-up is independent, and no
//! origin-index lookup is consulted to share the same upper inode.
//! Upper-authoritative sources already share one upper inode because they
//! are not split by separate copy-up.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L584-L635>
//!   (Linux `ovl_create_or_link` link path)

use crate::{
    fs::{
        fs_impls::overlayfs::{inode::OverlayInode, workdir::WorkdirTempRequest},
        vfs::{inode::RenameMode, path::Path},
    },
    prelude::*,
};

impl OverlayInode {
    /// Promotes the link source to upper authority and returns the shared
    /// upper real object's dentry-anchored [`Path`] that the new target hard
    /// link shares with the source; promotion errors propagate unchanged.
    pub(super) fn link_source(&self, old: &Arc<OverlayInode>) -> Result<Path> {
        old.ensure_upper_authority()?;
        let facts = old.facts_snapshot();
        let upper = facts.upper.ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the link source has no upper real object after promotion",
            )
        })?;
        upper.real_path()
    }

    /// Atomically replaces the published whiteout at `name` with a hard link
    /// to the shared source upper real object; on failure the staged hard
    /// link is removed best-effort.
    pub(super) fn link_over_whiteout(&self, name: &str, source_path: &Path) -> Result<()> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let temp = fs.create_workdir_temp(
            name,
            WorkdirTempRequest::Link {
                source: source_path.clone(),
            },
        )?;
        let workdir_path = self.workdir_root_path()?;
        if let Err(err) =
            workdir_path.rename(temp.name(), &upper_parent_path, name, RenameMode::Replace)
        {
            let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
            return Err(err);
        }
        Ok(())
    }
}
