// SPDX-License-Identifier: MPL-2.0

//! The create-object recipe family.
//!
//! One dispatcher routes on the fresh layer lookup of the target name: an
//! absent or opaque-hidden name creates directly in the upper parent, a
//! whiteout-hidden name replaces the whiteout through a completely prepared
//! private workdir temp, and a fresh positive target fails with `ESTALE`.
//! All three VFS create-family entries flow through one [`CreateOp`], so the
//! op's `object_type()` is the single op-to-type derivation and `SymLink`
//! can only enter through the atomic `create_symlink` entry.
//!
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                CreateOp, Lookup, NegativeLookup, OverlayInode, ReaddirCache,
                copyup::workdir::WorkdirTemp,
            },
            layer::RealObjectStack,
            real::RealObject,
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

impl OverlayInode {
    pub(super) fn create_object(
        &self,
        name: &str,
        op: &CreateOp<'_>,
        index: &mut Option<ReaddirCache>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc();
        match fs.lookup(self, name)? {
            Lookup::Negative(NegativeLookup::Absent) => self.create_upper_only(name, op, index),
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                self.create_over_whiteout(name, op, index)
            }
            Lookup::Positive(_) => Err(Error::new(Errno::ESTALE)),
        }
    }

    fn create_upper_only(
        &self,
        name: &str,
        op: &CreateOp<'_>,
        index: &mut Option<ReaddirCache>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc();
        let upper_parent = self.upper_parent_dentry()?;
        // `create_child_in` selects the single real-layer syscall per op.
        let new_upper = op.create_child_in(upper_parent, name)?;
        let new_facts = RealObjectStack::upper_only(RealObject::new(0, new_upper));
        let inode = fs.project_inode(&new_facts);
        // A freshly created upper-only object is not origin-preserved.
        self.readdir_cache_insert(name, op.object_type(), inode.ino(), false, index);
        Ok(inode)
    }

    fn create_over_whiteout(
        &self,
        name: &str,
        op: &CreateOp<'_>,
        index: &mut Option<ReaddirCache>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc();
        let upper_parent = self.upper_parent_dentry()?;
        let temp = fs.create_workdir_temp(name, op)?;
        let mut committed = false;
        let result = self.commit_create_over_whiteout(
            &fs,
            &temp,
            op.object_type(),
            upper_parent,
            name,
            index,
            &mut committed,
        );
        match result {
            Ok(inode) => Ok(inode),
            Err(err) => {
                if committed {
                    self.invalidate_readdir_cache(index);
                } else {
                    let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
                }
                Err(err)
            }
        }
    }

    /// Commits a whiteout-replacing temp into the upper parent.
    #[expect(clippy::too_many_arguments)]
    fn commit_create_over_whiteout(
        &self,
        fs: &Arc<OverlayFs>,
        temp: &WorkdirTemp,
        object_type: InodeType,
        upper_parent: &Arc<Dentry>,
        name: &str,
        index: &mut Option<ReaddirCache>,
        committed: &mut bool,
    ) -> Result<Arc<OverlayInode>> {
        // The opaque marker is written before the swap, so the whiteout is not deleted first.
        if object_type == InodeType::Dir {
            OverlayInode::set_opaque_marker(
                temp.dentry(),
                fs.policy().xattr_namespace(),
                fs.policy().can_store_private_xattr(),
                "the upper filesystem cannot store the opaque marker \
                 required for a directory over a whiteout",
            )?;
        }
        // Directories use Exchange plus a displaced unlink; other types use Replace.
        let published = if object_type.is_directory() {
            let published = fs.publish_temp(temp, upper_parent, name, RenameMode::Exchange)?;
            *committed = true;
            fs.workdir_root_dentry()?
                .as_dir_dentry_or_err()?
                .unlink(temp.name())?;
            published
        } else {
            let published = fs.publish_temp(temp, upper_parent, name, RenameMode::Replace)?;
            *committed = true;
            published
        };
        let new_facts = RealObjectStack::upper_only(RealObject::new(0, published));
        let inode = fs.project_inode(&new_facts);
        // A whiteout-replacing upper-only object is not origin-preserved.
        self.readdir_cache_insert(name, object_type, inode.ino(), false, index);
        Ok(inode)
    }
}
