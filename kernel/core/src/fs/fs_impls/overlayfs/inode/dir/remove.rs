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
            fs::{OverlayFs, mount::WhiteoutCapability},
            inode::{
                CreateOp, Lookup, OverlayInode, ReaddirCache, copyup::workdir::WorkdirTemp,
                xattr::XattrCopyPolicy,
            },
            real::RealObject,
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
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
        child_dentry: &Dentry,
        name: &str,
        kind: RemoveKind,
        index: &mut Option<ReaddirCache>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let lookup = fs.lookup(self, name)?;
        let target_inode = match lookup {
            Lookup::Positive(inode) => inode,
            Lookup::Negative(_) => return Err(Error::new(Errno::ENOENT)),
        };
        let target_upper = target_inode.upper.get();
        let target_lowers = &target_inode.lowers;

        // Remove the stale upper object first so its `ENOENT` becomes `ESTALE`, before the gates.
        let expected_has_upper = child_dentry
            .inode()
            .downcast_ref::<OverlayInode>()
            .is_some_and(|operation| operation.upper.get().is_some());
        if target_upper.is_none() && expected_has_upper {
            let upper_dir = self.upper_parent_dentry()?.as_dir_dentry_or_err()?;
            let result = match kind {
                RemoveKind::Rmdir => upper_dir.rmdir(name),
                RemoveKind::Unlink => upper_dir.unlink(name),
            };
            result.map_err(translate_stale_upper_enoent)?;
        }

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
                    // `NeedsRebuild` emptiness: fail conservatively with `ENOTEMPTY`.
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

        // Classify before any mutation: only a non-pure-upper removal publishes a whiteout.
        let is_pure_upper = match target_upper {
            Some(upper_obj) => {
                target_lowers.is_empty()
                    && !super::super::is_opaque_directory(upper_obj, namespace)?
            }
            None => false,
        };
        if !is_pure_upper && fs.policy().whiteout_capability() == WhiteoutCapability::Unsupported {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the removal \
                 cannot publish one",
            ));
        }

        let upper_parent = self.upper_parent_dentry()?;

        if is_pure_upper {
            // The gate ignores whiteouts, so sweep whiteout residue before the rmdir.
            if kind == RemoveKind::Rmdir {
                let target_upper_dir = target_upper.ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the pure-upper rmdir target has no upper real directory",
                    )
                })?;
                super::whiteout::cleanup_upper_whiteouts(target_upper_dir.dentry(), namespace)?;
            }
            let upper_dir = upper_parent.as_dir_dentry_or_err()?;
            let result = if kind == RemoveKind::Rmdir {
                upper_dir.rmdir(name)
            } else {
                upper_dir.unlink(name)
            };
            result.map_err(translate_stale_upper_enoent)?;
            self.readdir_cache_remove(name, index);
            self.clear_impure_marker(index, "remove");
            return Ok(());
        }

        let target_type = target_inode.type_();
        let replace_target = target_upper.map(|_| target_type);
        let clear_empty_temp = if kind == RemoveKind::Rmdir {
            match target_upper {
                Some(upper_obj) => {
                    let upper_names =
                        crate::fs::fs_impls::overlayfs::read_child_names(upper_obj.real_inode())?;
                    if upper_names.is_empty() {
                        None
                    } else {
                        let mode = upper_obj.real_inode().mode()?;
                        Some(fs.create_workdir_temp(
                            name,
                            &CreateOp::General {
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
        let result = self.commit_remove(
            &fs,
            clear_empty_temp.as_ref(),
            target_upper,
            name,
            upper_parent,
            replace_target,
            index,
            &mut committed,
        );
        match result {
            Ok(()) => {}
            Err(err) => {
                if committed {
                    self.invalidate_readdir_cache(index);
                } else if let Some((temp_name, kind)) = staged_temp {
                    let _ = fs.cleanup_workdir_temp(temp_name, kind);
                }
                return Err(err);
            }
        }
        self.clear_impure_marker(index, "remove");
        Ok(())
    }

    /// Commits the whiteout-publishing remove path.
    #[expect(clippy::too_many_arguments)]
    fn commit_remove(
        &self,
        fs: &Arc<OverlayFs>,
        clear_empty_temp: Option<&WorkdirTemp>,
        target_upper: Option<&RealObject>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        replace_target: Option<InodeType>,
        index: &mut Option<ReaddirCache>,
        committed: &mut bool,
    ) -> Result<()> {
        if let Some(temp) = clear_empty_temp {
            self.clear_empty_exchange(fs, target_upper, name, upper_parent, temp)?;
            *committed = true;
        }
        fs.publish_whiteout(upper_parent, name, replace_target)
            .map_err(|err| {
                if replace_target.is_some() {
                    translate_stale_upper_enoent(err)
                } else {
                    err
                }
            })?;
        *committed = true;
        self.finish_whiteout_cache(Some(name), index);
        Ok(())
    }

    /// Whiteout-hidden upper entries would make `publish_whiteout` fail `ENOTEMPTY`.
    fn clear_empty_exchange(
        &self,
        fs: &Arc<OverlayFs>,
        target_upper: Option<&RealObject>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        temp: &WorkdirTemp,
    ) -> Result<()> {
        let Some(upper_obj) = target_upper else {
            return Err(Error::with_message(
                Errno::EIO,
                "the clear-empty workdir temp has no upper directory",
            ));
        };
        let old_upper_dir = upper_obj.real_inode().clone();
        let namespace = fs.policy().xattr_namespace();
        // The opaque marker is written before the exchange so the name stays a search barrier.
        OverlayInode::set_opaque_marker(
            temp.dentry(),
            namespace,
            fs.policy().can_store_private_xattr(),
            "the upper filesystem cannot store the opaque marker \
             required for the clear-empty directory exchange",
        )?;
        // Xattrs are copied before owner/group/mode, so a non-owner rmdir cannot fail `EACCES`.
        OverlayInode::copy_eligible_xattrs(
            &old_upper_dir,
            temp,
            XattrCopyPolicy::BestEffort,
            namespace,
        )?;
        self.transfer_metadata(&old_upper_dir, temp)?;
        self.transfer_timestamps(&old_upper_dir, temp)?;
        fs.publish_temp(temp, upper_parent, name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        let workdir_dentry = fs.workdir_root_dentry()?;
        let workdir_dir = workdir_dentry.as_dir_dentry_or_err()?;
        match workdir_dir.lookup_child(temp.name()) {
            Ok(displaced) => {
                if let Err(cleanup_err) =
                    super::whiteout::cleanup_upper_whiteouts(&displaced, namespace)
                {
                    warn!(
                        "overlay clear-empty: the displaced-directory whiteout \
                         cleanup failed (residue, never a visible source): {:?}",
                        cleanup_err
                    );
                }
                if let Err(cleanup_err) = workdir_dir.rmdir(temp.name()) {
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

/// Maps a stale-target `ENOENT` to `ESTALE`, approximating a real dentry staleness check.
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
