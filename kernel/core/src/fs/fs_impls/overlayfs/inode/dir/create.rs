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
                CreateOp, Lookup, NegativeLookup, OverlayInode, OverlayInodeLockPayload,
                OverlayXattrType, copyup::workdir::WorkdirTemp,
            },
            layer::RealObjectStack,
            real::RealObject,
        },
        vfs::{inode::RenameMode, path::Dentry},
    },
    prelude::*,
};

impl OverlayInode {
    pub(super) fn create_object(
        &self,
        name: &str,
        op: &CreateOp<'_>,
        lock_payload: &mut OverlayInodeLockPayload,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc();
        match fs.lookup(self, name)? {
            Lookup::Negative(NegativeLookup::Absent) => {
                let upper = self.writable_upper();
                // `create_child_in` selects the single real-layer syscall per op.
                let new_upper = op.create_child_in(upper.dentry(), name)?;
                let inode = fs.project_inode(RealObjectStack::upper_only(
                    RealObject::new_upper(new_upper),
                ))?;
                // The new name belongs to the parent's next merge, so the snapshot that lacks it is stale.
                *lock_payload = None;
                Ok(inode)
            }
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                self.create_over_whiteout(name, op, lock_payload)
            }
            Lookup::Positive(_) => Err(Error::new(Errno::ESTALE)),
        }
    }

    fn create_over_whiteout(
        &self,
        name: &str,
        op: &CreateOp<'_>,
        lock_payload: &mut OverlayInodeLockPayload,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper();
        let upper_workdir = fs.upper_workdir_inuse();
        let object_type = op.object_type();
        // The staged create consumes its op, so the borrowed request is rebuilt by shape.
        let op = match op {
            CreateOp::General { kind, mode } => CreateOp::general(*kind, *mode),
            CreateOp::Symlink { target, mode } => CreateOp::symlink(target, *mode),
            CreateOp::Mknod { mode, node } => CreateOp::mknod(node, *mode),
        };
        let temp = upper_workdir.create_workdir_temp(name, op)?;
        let mut committed = false;
        let result = self.commit_create_over_whiteout(
            &fs,
            &temp,
            object_type,
            upper_parent.dentry(),
            name,
            lock_payload,
            &mut committed,
        );
        match result {
            Ok(inode) => Ok(inode),
            Err(err) => {
                if committed {
                    // The commit already replaced the name, so the parent's snapshot is stale.
                    *lock_payload = None;
                } else {
                    let _ = upper_workdir.cleanup_workdir_temp(temp.name(), temp.inode().type_());
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
        lock_payload: &mut OverlayInodeLockPayload,
        committed: &mut bool,
    ) -> Result<Arc<OverlayInode>> {
        let is_directory = object_type == InodeType::Dir;
        let upper_workdir = fs.upper_workdir_inuse();
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
        // Directories use Exchange plus a displaced unlink; other types use Replace.
        let published = if is_directory {
            let published = upper_workdir.publish_workdir_temp(
                temp,
                upper_parent,
                name,
                RenameMode::Exchange,
            )?;
            *committed = true;
            upper_workdir
                .workdir_workspace()?
                .as_dir_dentry_or_err()?
                .unlink(temp.name())?;
            published
        } else {
            let published = upper_workdir.publish_workdir_temp(
                temp,
                upper_parent,
                name,
                RenameMode::Replace,
            )?;
            *committed = true;
            published
        };
        let inode = fs.project_inode(RealObjectStack::upper_only(RealObject::new_upper(
            published,
        )))?;
        // The replaced name belongs to the parent's next merge, so the old snapshot is stale.
        *lock_payload = None;
        Ok(inode)
    }
}
