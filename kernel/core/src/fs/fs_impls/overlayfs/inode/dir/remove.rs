// SPDX-License-Identifier: MPL-2.0

//! The remove recipes: the shared unlink/rmdir recipe on [`OverlayInode`],
//! parameterized by [`RemoveKind`].
//!
//! [`RemoveKind::Unlink`] and [`RemoveKind::Rmdir`] name the operation;
//! `remove_target` is the shared recipe, with `clear_empty_exchange` and
//! `translate_stale_upper_enoent` as helpers. The **clear-empty exchange**
//! (`clear_empty_exchange`) is the rmdir path for a target that has an
//! upper directory with entries: the upper directory is exchanged with a
//! prepared empty workdir temp so the whiteout can be published without
//! `ENOTEMPTY`.
//!
//! Lock contract: this module enters the per-object copy-up coordination
//! lock only through the copy-up step of `check_permission`, which runs
//! before the parent transaction lock is taken; the whiteout cache lock is
//! entered only through `publish_whiteout`, while holding the parent
//! directory transaction lock.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L763-L807>
//!   (Linux `ovl_remove_and_whiteout` whiteout-publish removal)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L809-L859>
//!   (Linux `ovl_remove_upper` direct upper removal)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/namei.c#L1418-L1480>
//!   (Linux `ovl_lower_positive` lower-presence check)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L758>
//!   (Linux `ovl_matches_upper` stale-upper check)

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{
                Lookup, OverlayInode, ReaddirIndex,
                copyup::workdir::{WorkdirTemp, WorkdirTempRequest},
                xattr::XattrCopyPolicy,
            },
            layer::RealObjectStack,
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Path,
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RemoveKind {
    Unlink,
    Rmdir,
}

impl OverlayInode {
    pub(super) fn remove_target(
        &self,
        name: &str,
        kind: RemoveKind,
        index: &mut Option<ReaddirIndex>,
    ) -> Result<()> {
        let fs = self.fs_arc()?;
        let prefix = fs.policy().xattr_prefix();
        let lookup = fs.lookup(self, name)?;
        if self.is_stale_upper(name, &lookup, index) {
            return Err(translate_stale_upper_enoent(Error::with_message(
                Errno::ENOENT,
                "the overlay target became stale behind the overlay",
            )));
        }
        let target_inode = match lookup {
            Lookup::Positive(inode) => inode,
            Lookup::Negative(_) => return Err(Error::new(Errno::ENOENT)),
        };
        let target_facts = target_inode.real_object_stack();

        if kind == RemoveKind::Rmdir {
            match target_inode.visible_child_count() {
                Ok(0) => {}
                Ok(_) => {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory is not empty",
                    ));
                }
                Err(err) if err.error() == Errno::ENOTDIR => {
                    return Err(err);
                }
                Err(_) => {
                    // Emptiness unresolvable (`NeedsRebuild`): fail
                    // conservatively with `ENOTEMPTY`, never an upper-only
                    // emptiness guess.
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory emptiness could not be verified",
                    ));
                }
            }
        } else if target_inode.type_().is_directory() {
            return Err(Error::with_message(
                Errno::EISDIR,
                "a directory cannot be unlinked",
            ));
        }

        let is_pure_upper = match target_facts.upper.as_ref() {
            Some(upper_obj) => {
                target_facts.lowers.is_empty()
                    && !super::super::is_opaque_directory(upper_obj, prefix)?
            }
            None => false,
        };

        let upper_parent_path = self.upper_parent_path()?;

        if is_pure_upper {
            // The visible-emptiness gate does not count whiteouts, so a
            // pure-upper rmdir may still hold physical whiteout residue —
            // sweep it before the physical rmdir.
            if kind == RemoveKind::Rmdir {
                let target_upper_dir = target_facts.upper.as_ref().ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the pure-upper rmdir target has no upper real directory",
                    )
                })?;
                super::whiteout::cleanup_upper_whiteouts(
                    &fs.real_object_path(target_upper_dir),
                    prefix,
                )?;
            }
            let result = if kind == RemoveKind::Rmdir {
                upper_parent_path.rmdir(name)
            } else {
                upper_parent_path.unlink(name)
            };
            result.map_err(translate_stale_upper_enoent)?;
            self.readdir_index_remove(name, index);
            self.refresh_impure_marker_best_effort(index, "remove");
            return Ok(());
        }

        let target_type = target_inode.type_();
        let replace_target = target_facts.upper.as_ref().map(|_| target_type);
        let clear_empty_temp = if kind == RemoveKind::Rmdir {
            match target_facts.upper.as_ref() {
                Some(upper_obj) => {
                    let upper_names =
                        crate::fs::fs_impls::overlayfs::read_child_names(upper_obj.real_inode())?;
                    if upper_names.is_empty() {
                        None
                    } else {
                        let mode = upper_obj.real_inode().mode()?;
                        Some(fs.create_workdir_temp(
                            name,
                            WorkdirTempRequest::Create {
                                kind: InodeType::Dir,
                                mode,
                            },
                        )?)
                    }
                }
                None => None,
            }
        } else {
            None
        };
        let staged_temp = clear_empty_temp
            .as_ref()
            .map(|temp| (temp.name(), temp.kind()));
        let mut committed = false;
        let result: Result<()> = (|| {
            if let Some(temp) = clear_empty_temp.as_ref() {
                self.clear_empty_exchange(&fs, &target_facts, name, &upper_parent_path, temp)?;
                committed = true;
            }
            fs.publish_whiteout(&upper_parent_path, name, replace_target)
                .map_err(|err| {
                    if replace_target.is_some() {
                        translate_stale_upper_enoent(err)
                    } else {
                        err
                    }
                })?;
            committed = true;
            self.finish_whiteout_index(Some(name), index);
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(err) => {
                if committed {
                    self.invalidate_readdir_index(index);
                } else if let Some((temp_name, kind)) = staged_temp {
                    let _ = fs.cleanup_workdir_temp(temp_name, kind);
                }
                return Err(err);
            }
        }
        self.refresh_impure_marker_best_effort(index, "remove");
        Ok(())
    }

    /// Needed because whiteout-hidden upper entries would make
    /// `publish_whiteout` fail with `ENOTEMPTY`.
    fn clear_empty_exchange(
        &self,
        fs: &Arc<OverlayFs>,
        target_facts: &RealObjectStack,
        name: &str,
        upper_parent_path: &Path,
        temp: &WorkdirTemp,
    ) -> Result<()> {
        let Some(upper_obj) = target_facts.upper.as_ref() else {
            return Err(Error::with_message(
                Errno::EIO,
                "the clear-empty workdir temp has no upper directory",
            ));
        };
        let old_upper_dir = upper_obj.real_inode().clone();
        let prefix = fs.policy().xattr_prefix();
        let workdir_path = self.workdir_root_path()?;
        // The opaque marker and the transfers target the temp itself: the
        // temp's own dentry reconstructs its dcache-anchored workdir path.
        let temp_path = Path::new(workdir_path.mount_node().clone(), temp.dentry().clone());
        // The opaque marker keeps the name a lower-search barrier at every
        // instant of the swap (crash window included), so it is written on
        // the temp before the exchange as part of complete preparation.
        fs.set_opaque_marker(
            &temp_path,
            "the upper filesystem cannot store the opaque marker \
             required for the clear-empty directory exchange",
        )?;
        // The xattr copy runs before owner/group/mode are applied, while the
        // temp is still owned by the creating task, so a non-owner rmdir
        // carrying xattrs cannot fail `EACCES` on the temp `set_xattr`.
        OverlayInode::copy_eligible_xattrs(
            &old_upper_dir,
            temp,
            XattrCopyPolicy::BestEffort,
            prefix,
        )?;
        self.transfer_metadata(&old_upper_dir, temp)?;
        self.transfer_timestamps(&old_upper_dir, temp)?;
        fs.publish_temp(temp, upper_parent_path, name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        match super::super::super::lookup_child_path(&workdir_path, temp.name()) {
            Ok(displaced_path) => {
                if let Err(cleanup_err) =
                    super::whiteout::cleanup_upper_whiteouts(&displaced_path, prefix)
                {
                    warn!(
                        "overlay clear-empty: the displaced-directory whiteout \
                         cleanup failed (residue, never a visible source): {:?}",
                        cleanup_err
                    );
                }
                if let Err(cleanup_err) = workdir_path.rmdir(temp.name()) {
                    warn!(
                        "overlay clear-empty: workdir cleanup of the displaced \
                         directory {:?} failed (residue, never a visible source): {:?}",
                        temp.name(),
                        cleanup_err
                    );
                }
            }
            Err(reobserve_err) => {
                warn!(
                    "overlay clear-empty: re-observation of the displaced \
                     directory {:?} failed (residue, never a visible source): {:?}",
                    temp.name(),
                    reobserve_err
                );
            }
        }
        Ok(())
    }
}

/// Translating stale-target `ENOENT` into `ESTALE` is an intentional
/// approximation of a real dentry-level staleness check: a faithful
/// VFS-level dentry check would require a breaking VFS change.
fn translate_stale_upper_enoent(err: Error) -> Error {
    if err.error() == Errno::ENOENT {
        Error::with_message(
            Errno::ESTALE,
            "the upper object at the target name became stale",
        )
    } else {
        err
    }
}
