// SPDX-License-Identifier: MPL-2.0

//! The create-object recipe family.
//!
//! One dispatcher routes on the fresh layer lookup of the target name: an
//! absent or opaque-hidden name creates directly in the upper parent, a
//! whiteout-hidden name replaces the whiteout through a completely prepared
//! private workdir temp, and a fresh positive target fails with `ESTALE`.
//! All three VFS create-family entries flow through one [`CreateOp`], and
//! `SymLink` can only enter through the atomic `create_symlink` entry.

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                CreateOp, Lookup, NegativeLookup, OverlayInode, OverlayInodeLockGuard,
                OverlayXattrType, copyup::workdir::WorkdirTemp,
            },
            real::{RealObject, RealObjectStack},
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

impl OverlayInode {
    /// Serves as the single create-family entry; `CreateOp` encodes the VFS entry that produced it.
    pub(in super::super) fn create_impl(
        &self,
        self_dentry: &Dentry,
        name: &str,
        op: CreateOp<'_>,
    ) -> Result<Arc<dyn Inode>> {
        let fs = self.fs_arc();
        // The receiver promotes before its lock, so the created name gets a real upper entry.
        self.writable_real_object(self_dentry)?;
        let projected: Arc<dyn Inode> = self.create_object(&fs, name, op)?;
        Ok(projected)
    }

    fn create_object(
        &self,
        fs: &OverlayFs,
        name: &str,
        op: CreateOp<'_>,
    ) -> Result<Arc<OverlayInode>> {
        let mut dir_guard = self.lock();
        match fs.lookup(self, name)? {
            Lookup::Negative(NegativeLookup::Absent) => {
                let upper = self.writable_upper();
                // `create_child_in` selects the single real-layer syscall per op.
                let new_upper = op.create_child_in(upper.dentry(), name)?;
                let inode = fs.project_inode(RealObjectStack::upper_only(
                    RealObject::new_upper(new_upper),
                ))?;
                // The new name belongs to the parent's next merge, so the snapshot that lacks it
                // is stale.
                *dir_guard = None;
                Ok(inode)
            }
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                self.create_over_whiteout(fs, name, op, &mut dir_guard)
            }
            Lookup::Positive(_) => return_errno!(Errno::ESTALE),
        }
    }

    fn create_over_whiteout(
        &self,
        fs: &OverlayFs,
        name: &str,
        op: CreateOp<'_>,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<Arc<OverlayInode>> {
        let upper_workdir = fs.upper_workdir_inuse();
        let object_type = op.object_type();
        let temp = upper_workdir.create_workdir_temp(name, op)?;
        let temp_name = if object_type == InodeType::Dir {
            Some(String::from(temp.name()))
        } else {
            None
        };
        let published = self.commit_create_over_whiteout(fs, temp, name)?;
        // The published name belongs to the parent's next merge, so the snapshot that lacks it
        // is stale.
        **dir_guard = None;
        if let Some(temp_name) = temp_name {
            upper_workdir
                .workdir_workspace()?
                .as_dir_dentry_or_err()?
                .unlink(&temp_name)?;
        }
        fs.project_inode(RealObjectStack::upper_only(RealObject::new_upper(
            published,
        )))
    }

    /// Commits a whiteout-replacing temp into the upper parent.
    fn commit_create_over_whiteout(
        &self,
        fs: &OverlayFs,
        temp: WorkdirTemp,
        name: &str,
    ) -> Result<Arc<Dentry>> {
        let upper_parent = self.writable_upper().dentry();
        let is_directory = temp.inode().type_() == InodeType::Dir;
        // The opaque marker is written before the swap, so the whiteout is not deleted first.
        if is_directory {
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the opaque marker \
                     required for a directory over a whiteout",
                ));
            }
            OverlayXattrType::Opaque.set_value_on(
                temp.dentry(),
                fs.policy().xattr_namespace(),
                None,
            )?;
        }
        let published = temp.dentry().clone();
        if is_directory {
            temp.publish(upper_parent, name, RenameMode::Exchange)?;
        } else {
            temp.publish(upper_parent, name, RenameMode::Replace)?;
        }
        Ok(published)
    }
}
