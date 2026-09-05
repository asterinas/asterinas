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
//! Lock contract: this module enters the promoted object's transaction lock
//! only through the copy-up step of `check_permission`, which runs before the
//! parent transaction lock is taken; the removal additionally takes the
//! removed object's own transaction lock (a parent-to-descendant edge) and
//! sets that object's name-taken latch only once the real removal has
//! succeeded or committed. The whiteout cache lock is entered only through
//! `publish_whiteout`, while holding the parent directory transaction lock.
//! The whiteout classification probes the lower layers inside that same
//! parent lock, and it is resolved before the emptiness gate, which runs
//! under the already-held target guard and only for the targets whose
//! classification asks for it; an emptiness check that cannot answer
//! propagates its own error instead of folding it into `ENOTEMPTY`.
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
                CreateOp, Lookup, OverlayInode, OverlayInodeLockPayload, OverlayXattrType,
                copyup::workdir::WorkdirTemp,
            },
            real::{RealObject, read_child_names},
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
        lock_payload: &mut OverlayInodeLockPayload,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        // Only this comparison sees a behind-the-back upper removal; the latches cannot.
        let lookup = fs.lookup(self, name)?;
        let target_inode = match lookup {
            Lookup::Positive(inode) => inode,
            Lookup::Negative(_) => return Err(Error::new(Errno::ENOENT)),
        };
        let target_upper = target_inode.upper.get();

        // Remove the stale upper object first so its `ENOENT` becomes `ESTALE`, before the gates.
        let expected_has_upper = child_dentry
            .inode()
            .downcast_ref::<OverlayInode>()
            .is_some_and(|operation| operation.upper.get().is_some());
        if target_upper.is_none() && expected_has_upper {
            let upper = self.writable_upper();
            let upper_dir = upper.dentry().as_dir_dentry_or_err()?;
            let result = match kind {
                RemoveKind::Rmdir => upper_dir.rmdir(name),
                RemoveKind::Unlink => upper_dir.unlink(name),
            };
            result.map_err(translate_stale_upper_enoent)?;
        }

        // Classified before the gates: the verdict decides both the gate and the whiteout branch.
        let lower_positive = if target_upper.is_some() {
            self.has_lower_entry(name, namespace)?
        } else {
            // Without an upper the name cannot be pure upper, and no gate reads the probe.
            false
        };
        let is_pure_upper = match target_upper {
            Some(upper_obj) => {
                !lower_positive
                    && !super::super::is_opaque_directory(upper_obj.dentry(), namespace)?
            }
            None => false,
        };

        // The target's own lock covers the gate and removal, so a promotion round sees it.
        let _target_guard = target_inode.lock();

        if kind == RemoveKind::Rmdir {
            if lower_positive || !is_pure_upper {
                // The guard is already held, so the emptiness answer takes and releases no lock.
                if !target_inode.is_empty_dir()? {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory is not empty",
                    ));
                }
            }
        } else if target_inode.type_().is_directory() {
            return Err(Error::with_message(
                Errno::EISDIR,
                "a directory cannot be unlinked",
            ));
        }

        if !is_pure_upper && fs.policy().whiteout_capability() == WhiteoutCapability::Unsupported {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the removal \
                 cannot publish one",
            ));
        }

        let upper_parent = self.writable_upper();

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
            let upper_dir = upper_parent.dentry().as_dir_dentry_or_err()?;
            let result = if kind == RemoveKind::Rmdir {
                upper_dir.rmdir(name)
            } else {
                upper_dir.unlink(name)
            };
            result.map_err(translate_stale_upper_enoent)?;
            target_inode.mark_name_taken();
            // The removed name is gone from the next merge, so the parent's snapshot is stale.
            *lock_payload = None;
            return Ok(());
        }

        let target_type = target_inode.type_();
        let replace_target = target_upper.map(|_| target_type);
        let clear_empty_temp = if kind == RemoveKind::Rmdir {
            match target_upper {
                Some(upper_obj) => {
                    let upper_names = read_child_names(upper_obj.real_inode())?;
                    if upper_names.is_empty() {
                        None
                    } else {
                        let mode = upper_obj.real_inode().mode()?;
                        Some(fs.upper_workdir_inuse().create_workdir_temp(
                            name,
                            CreateOp::General {
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
            .map(|temp| (temp.name(), temp.inode().type_()));
        let mut committed = false;
        let result = self.commit_remove(
            &fs,
            clear_empty_temp.as_ref(),
            target_upper,
            name,
            upper_parent.dentry(),
            replace_target,
            lock_payload,
            &mut committed,
        );
        match result {
            Ok(()) => {
                target_inode.mark_name_taken();
            }
            Err(err) => {
                if committed {
                    // A partially committed removal already gave the name up.
                    target_inode.mark_name_taken();
                    *lock_payload = None;
                } else if let Some((temp_name, kind)) = staged_temp {
                    let _ = fs
                        .upper_workdir_inuse()
                        .cleanup_workdir_temp(temp_name, kind);
                }
                return Err(err);
            }
        }
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
        lock_payload: &mut OverlayInodeLockPayload,
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
        // The published whiteout is a new upper name, so the parent's snapshot is stale.
        *lock_payload = None;
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
        if !fs.policy().can_store_private_xattr() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the opaque marker \
                 required for the clear-empty directory exchange",
            ));
        }
        OverlayXattrType::Opaque.set_value_on(temp.dentry(), namespace, None)?;
        // Xattrs are copied before owner/group/mode, so a non-owner rmdir cannot fail `EACCES`.
        self.copy_eligible_xattrs(&old_upper_dir, temp, true)?;
        upper_obj.copy_metadata_to(temp.dentry())?;
        upper_obj.copy_timestamps_to(temp.dentry())?;
        fs.upper_workdir_inuse()
            .publish_workdir_temp(temp, upper_parent, name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        let workspace = fs.upper_workdir_inuse().workdir_workspace()?;
        let workdir_dir = workspace.as_dir_dentry_or_err()?;
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
