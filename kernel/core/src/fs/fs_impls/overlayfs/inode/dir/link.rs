// SPDX-License-Identifier: MPL-2.0

//! The link recipe.
//!
//! Lock contract: source promotion runs before the parent transaction lock
//! is taken, so copy-up is entered with no overlay lock held and takes the
//! publication parent's lock before the promoted object's.
//!
//! Owns [`OverlayInode::link_over_whiteout`]; the `Inode::link` entry composes
//! it around that transaction lock; temp cleanup on failure is explicit and
//! fallible, never an RAII rollback.
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
        fs_impls::overlayfs::inode::OverlayInode,
        vfs::{inode::RenameMode, path::Dentry},
    },
    prelude::*,
};

impl OverlayInode {
    pub(super) fn link_over_whiteout(&self, name: &str, source: &Arc<Dentry>) -> Result<()> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper();
        let upper_workdir = fs.upper_workdir_inuse();
        let temp = upper_workdir.create_workdir_link_temp(name, source)?;
        if let Err(err) = upper_workdir.publish_workdir_temp(
            &temp,
            upper_parent.dentry(),
            name,
            RenameMode::Replace,
        ) {
            let _ = upper_workdir.cleanup_workdir_temp(temp.name(), temp.inode().type_());
            return Err(err);
        }
        Ok(())
    }
}
