// SPDX-License-Identifier: MPL-2.0

//! The create-object recipe family.
//!
//! One dispatcher routes on the fresh layer lookup of the target name: an
//! absent or opaque-hidden name creates directly in the upper parent, a
//! whiteout-hidden name replaces the whiteout through a completely prepared
//! private workdir temp, and a fresh positive target fails with `ESTALE`.
//! The atomic `create_symlink_impl` entry follows the same dispatch shape,
//! creating the symlink with its target in one step (`new_symlink_child` in
//! the absent branch, a `WorkdirTempRequest::Symlink` temp plus the publish
//! mechanism in the over-whiteout branch).
//!
use crate::{
    fs::{
        file::{InodeMode, InodeType, Permission},
        fs_impls::overlayfs::{
            inode::{
                Lookup, NegativeLookup, OverlayInode, ReaddirIndex,
                copyup::workdir::WorkdirTempRequest, mknod_object_type, permission::CopyUpOrigin,
            },
            layer::RealObjectStack,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::{Dentry, Path},
        },
    },
    prelude::*,
};

impl OverlayInode {
    pub(super) fn create_object(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
        index: &mut Option<ReaddirIndex>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let lookup = fs.lookup(self, name)?;
        match lookup {
            Lookup::Negative(NegativeLookup::Absent) => {
                self.create_upper_only(name, type_, mode, mknod_type, index)
            }
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                self.create_over_whiteout(name, type_, mode, mknod_type, index)
            }
            Lookup::Positive(_) => Err(Error::new(Errno::ESTALE)),
        }
    }

    fn create_upper_only(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
        index: &mut Option<ReaddirIndex>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let object_type = mknod_type.as_ref().map(mknod_object_type).unwrap_or(type_);
        let new_upper_path = match mknod_type {
            Some(mknod) => upper_parent_path.mknod(name, mode, mknod)?,
            None => upper_parent_path.new_child(name, type_, mode)?,
        };
        let new_facts =
            RealObjectStack::upper_only(fs.layer_stack().upper_child_real_object(&new_upper_path)?);
        let inode = fs.project_inode(&new_facts);
        self.readdir_index_insert(name, inode.clone(), object_type, index);
        Ok(inode)
    }

    fn create_over_whiteout(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        mknod_type: Option<MknodType>,
        index: &mut Option<ReaddirIndex>,
    ) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        let upper_parent_path = self.upper_parent_path()?;
        let object_type = mknod_type.as_ref().map(mknod_object_type).unwrap_or(type_);
        let temp = match &mknod_type {
            Some(node) => fs.create_workdir_temp(name, WorkdirTempRequest::Mknod { mode, node })?,
            None => {
                fs.create_workdir_temp(name, WorkdirTempRequest::Create { kind: type_, mode })?
            }
        };
        let workdir_path = self.workdir_root_path()?;
        // The opaque marker targets the temp itself: the temp's own dentry
        // reconstructs its dcache-anchored workdir path.
        let temp_path = Path::new(workdir_path.mount_node().clone(), temp.dentry().clone());
        let mut committed = false;
        let result: Result<Arc<OverlayInode>> = (|| {
            if object_type == InodeType::Dir {
                // Part of complete publication: the opaque marker is written
                // on the temp before the atomic swap, so the whiteout is
                // never deleted first.
                fs.set_opaque_marker(
                    &temp_path,
                    "the upper filesystem cannot store the opaque marker \
                     required for a directory over a whiteout",
                )?;
            }
            let published_path = if object_type.is_directory() {
                let published =
                    fs.publish_temp(&temp, &upper_parent_path, name, RenameMode::Exchange)?;
                committed = true;
                workdir_path.unlink(temp.name())?;
                published
            } else {
                let published =
                    fs.publish_temp(&temp, &upper_parent_path, name, RenameMode::Replace)?;
                committed = true;
                published
            };
            let new_facts = RealObjectStack::upper_only(
                fs.layer_stack().upper_child_real_object(&published_path)?,
            );
            let inode = fs.project_inode(&new_facts);
            self.readdir_index_insert(name, inode.clone(), object_type, index);
            Ok(inode)
        })();
        match result {
            Ok(inode) => Ok(inode),
            Err(err) => {
                if committed {
                    self.invalidate_readdir_index(index);
                } else {
                    let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
                }
                Err(err)
            }
        }
    }

    /// The atomic symlink entry: the symlink is created with its target in
    /// one step, so no separate target write (the retired `write_link` flow)
    /// can leave a dangling entry. The dispatch mirrors `create_object`:
    /// absent names create directly in the upper parent, whiteout-hidden
    /// names publish a prepared `Symlink` temp with `Replace`, and a fresh
    /// positive target fails with `ESTALE`.
    pub(in super::super) fn create_symlink_impl(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.check_mutating_permission(
            CopyUpOrigin::Operation(self_dentry),
            Permission::MAY_WRITE,
        )?;
        let fs = self.fs_arc()?;
        let mut dir_guard = self.lock_dir_transaction();
        match fs.lookup(self, name)? {
            Lookup::Negative(NegativeLookup::Absent) => {
                let upper_parent_path = self.upper_parent_path()?;
                let new_upper_path = upper_parent_path.new_symlink_child(name, target, mode)?;
                let new_facts = RealObjectStack::upper_only(
                    fs.layer_stack().upper_child_real_object(&new_upper_path)?,
                );
                let inode = fs.project_inode(&new_facts);
                self.readdir_index_insert(name, inode.clone(), InodeType::SymLink, &mut dir_guard);
                Ok(inode)
            }
            Lookup::Negative(NegativeLookup::HiddenByWhiteout) => {
                let upper_parent_path = self.upper_parent_path()?;
                let temp = fs.create_workdir_temp(
                    name,
                    WorkdirTempRequest::Symlink {
                        target: String::from(target),
                        mode,
                    },
                )?;
                let mut committed = false;
                let result: Result<Arc<OverlayInode>> = (|| {
                    let published_path =
                        fs.publish_temp(&temp, &upper_parent_path, name, RenameMode::Replace)?;
                    committed = true;
                    let new_facts = RealObjectStack::upper_only(
                        fs.layer_stack().upper_child_real_object(&published_path)?,
                    );
                    let inode = fs.project_inode(&new_facts);
                    self.readdir_index_insert(
                        name,
                        inode.clone(),
                        InodeType::SymLink,
                        &mut dir_guard,
                    );
                    Ok(inode)
                })();
                match result {
                    Ok(inode) => Ok(inode),
                    Err(err) => {
                        if committed {
                            self.invalidate_readdir_index(&mut dir_guard);
                        } else {
                            let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
                        }
                        Err(err)
                    }
                }
            }
            Lookup::Positive(_) => Err(Error::new(Errno::ESTALE)),
        }
    }
}
