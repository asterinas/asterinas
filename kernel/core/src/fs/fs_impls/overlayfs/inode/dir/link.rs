// SPDX-License-Identifier: MPL-2.0

//! The link recipe.
//!
//! Lock contract: source promotion runs before the parent transaction lock
//! is taken, so this module never enters the per-object copy-up coordination
//! lock while holding a parent lock.
//!
//! Owns [`OverlayInode::link_source`] and [`OverlayInode::link_over_whiteout`];
//! the `Inode::link` entry composes them around that transaction lock;
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
        fs_impls::overlayfs::inode::{OverlayInode, copyup::workdir::WorkdirTempRequest},
        vfs::{
            inode::RenameMode,
            path::{Dentry, Path},
        },
    },
    prelude::*,
};

impl OverlayInode {
    pub(super) fn link_source(&self, old: &Arc<OverlayInode>, old_dentry: &Dentry) -> Result<Path> {
        let fs = self.fs_arc()?;
        old.copy_up_at(old_dentry)?;
        let upper = old.upper.get().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the link source has no upper real object after promotion",
            )
        })?;
        Ok(fs.real_object_path(upper))
    }

    pub(super) fn link_over_whiteout(&self, name: &str, source_path: &Path) -> Result<()> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let temp = fs.create_workdir_temp(
            name,
            WorkdirTempRequest::Link {
                source: source_path.clone(),
            },
        )?;
        if let Err(err) = fs.publish_temp(&temp, &upper_parent_path, name, RenameMode::Replace) {
            let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
            return Err(err);
        }
        Ok(())
    }
}
