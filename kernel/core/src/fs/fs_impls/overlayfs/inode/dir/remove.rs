// SPDX-License-Identifier: MPL-2.0

//! The remove recipes: the shared unlink/rmdir recipe on [`OverlayInode`],
//! parameterized by [`RemoveKind`].
//!
//! An unlink or rmdir removes a name from the merged view: afterwards that name resolves
//! negatively for every lookup, while the lower layers keep whatever objects they held.
//!
//! Where the upper entry is the name's only witness, the removal deletes it outright; everywhere
//! else a published whiteout hides the name, with the upper directory first replaced through a
//! prepared empty workdir temp whenever it still holds entries.

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            inode::{
                CreateOp, Lookup, OverlayInode, OverlayInodeLockGuard, OverlayXattrType,
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
enum RemoveKind {
    Unlink,
    Rmdir,
}

/// Records the action a removal performs: one variant per upper state that survives the gates.
#[derive(Clone, Copy, Debug)]
enum RemoveAction {
    /// Applies where the upper entry is the name's only witness: the removal deletes it.
    DeletePureUpper,
    /// Applies where the upper holds no entry: a whiteout hides the name the lower layers carry.
    WhiteoutLowerOnly,
    /// Applies where the upper entry is not the name's only witness: a whiteout must displace it.
    ReplaceUpperWithWhiteout,
}

/// Fixes the projection and the name of one removal as one pair.
///
/// It is built inside each step that needs it, never carried across a lock boundary, and holds no
/// layer fact, so nothing it pairs can go stale while the locks are taken.
struct RemoveTarget {
    /// Holds the name's fresh projection, resolved by this request's own lookup.
    inode: Arc<OverlayInode>,
    /// Carries the name, derived once from the caller's dentry and owned here.
    name: String,
}

impl OverlayInode {
    /// Serves as the unlink entry the VFS calls: it resolves the removed child's parent and runs
    /// the removal.
    pub(in super::super) fn unlink_impl(&self, child_dentry: &Dentry) -> Result<()> {
        // The take point promotes the parent, sourcing its target from the removed entry's parent.
        let parent_dentry = child_dentry
            .parent()
            .expect("the unlinked child has no parent dentry");
        self.refuse_removal_early(child_dentry, RemoveKind::Unlink)?;
        self.writable_real_object(&parent_dentry)?;
        self.remove_target(child_dentry, RemoveKind::Unlink)
    }

    /// Serves as the rmdir entry the VFS calls: it resolves the removed child's parent and runs
    /// the removal.
    pub(in super::super) fn rmdir_impl(&self, child_dentry: &Dentry) -> Result<()> {
        let parent_dentry = child_dentry
            .parent()
            .expect("the removed directory has no parent dentry");
        self.refuse_removal_early(child_dentry, RemoveKind::Rmdir)?;
        self.writable_real_object(&parent_dentry)?;
        self.remove_target(child_dentry, RemoveKind::Rmdir)
    }

    /// Answers the verdicts before the parent is promoted.
    ///
    /// A doomed removal pays no copy-up. The name keeps the family's historical `removal` infix,
    /// its one spelling exception.
    fn refuse_removal_early(&self, child_dentry: &Dentry, kind: RemoveKind) -> Result<()> {
        let target = self.resolve_remove_target(child_dentry)?;
        let target_guard = target.inode.lock();
        let _ = self.decide_remove_action(&target, kind, &target_guard)?;
        Ok(())
    }

    /// Sends the removal of one name to the arm its verdict names.
    fn remove_target(&self, child_dentry: &Dentry, kind: RemoveKind) -> Result<()> {
        let mut dir_guard = self.lock();
        let target = self.resolve_remove_target(child_dentry)?;
        let target_guard = target.inode.lock();
        let action = self.decide_remove_action(&target, kind, &target_guard)?;
        match action {
            RemoveAction::DeletePureUpper => self.delete_pure_upper(&target, kind, &mut dir_guard),
            RemoveAction::WhiteoutLowerOnly => self.whiteout_lower_only(&target, &mut dir_guard),
            RemoveAction::ReplaceUpperWithWhiteout => {
                self.replace_upper_with_whiteout(&target, kind, &mut dir_guard)
            }
        }
    }

    fn resolve_remove_target(&self, child_dentry: &Dentry) -> Result<RemoveTarget> {
        let name = child_dentry.name();
        let lookup = self.fs_arc().lookup(self, &name)?;
        let inode = match lookup {
            Lookup::Positive(inode) => inode,
            Lookup::Negative(_) => return_errno!(Errno::ENOENT),
        };
        // The caller's dentry and the fresh projection disagree on the name's upper entry, so the
        // caller's view of the name is stale and the removal stops here instead of acting on it.
        if inode.is_stale_upper_target(child_dentry) {
            return Err(Error::with_message(
                Errno::ESTALE,
                "the upper object at the target name became stale",
            ));
        }
        Ok(RemoveTarget { inode, name })
    }

    /// Decides the action one removal performs from the target's upper state and its lower probe.
    fn decide_remove_action(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        target_guard: &OverlayInodeLockGuard<'_>,
    ) -> Result<RemoveAction> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let target_upper = target.inode.upper.get();
        // Classified before the gates: the verdict decides both the gate and the whiteout branch.
        let lower_positive = if target_upper.is_some() {
            // An unanswerable probe counts as a hit: a removal must never leave a name behind.
            match self.lower_entry(&target.name) {
                Ok(found) => found.is_some(),
                Err(_) => true,
            }
        } else {
            // Without an upper the name cannot be pure upper, and no gate reads the probe.
            false
        };
        let is_pure_upper = match target_upper {
            Some(upper_obj) => {
                !lower_positive
                    // The opaque marker only ever sits on a directory, so a non-directory target
                    // asks no opaque question; the conjunct is the precondition, not a re-check.
                    && !(target.inode.type_().is_directory()
                        && OverlayXattrType::Opaque.is_positive_on(upper_obj.dentry(), namespace)?)
            }
            None => false,
        };

        // The target's own lock covers the gate and removal, so a promotion round sees it.
        if kind == RemoveKind::Rmdir {
            if !target.inode.type_().is_directory() {
                return Err(Error::with_message(
                    Errno::ENOTDIR,
                    "a non-directory cannot be removed as a directory",
                ));
            }
            if lower_positive || !is_pure_upper {
                // The merged view's emptiness, whiteouts excluded, decides `ENOTEMPTY`.
                // The guard is already held, so the emptiness answer takes and releases no lock.
                if !target.inode.is_empty_dir(target_guard)? {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory is not empty",
                    ));
                }
            }
        } else if target.inode.type_().is_directory() {
            return Err(Error::with_message(
                Errno::EISDIR,
                "a directory cannot be unlinked",
            ));
        }

        if !is_pure_upper && !fs.policy().can_express_whiteout() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the removal \
                 cannot publish one",
            ));
        }
        Ok(if is_pure_upper {
            RemoveAction::DeletePureUpper
        } else if target_upper.is_some() {
            RemoveAction::ReplaceUpperWithWhiteout
        } else {
            RemoveAction::WhiteoutLowerOnly
        })
    }

    /// Deletes the upper entry itself, all a removal needs where the upper alone holds the name.
    fn delete_pure_upper(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let target_upper = target.inode.upper.get();
        let upper_parent = self.writable_upper();
        // The real layer counts whiteouts, so the residue they leave is swept before the rmdir.
        if kind == RemoveKind::Rmdir {
            let target_upper_dir = target_upper.ok_or_else(|| {
                Error::with_message(
                    Errno::EIO,
                    "the pure-upper rmdir target has no upper real directory",
                )
            })?;
            fs.sweep_whiteouts(target_upper_dir.dentry())?;
        }
        let upper_dir = upper_parent.dentry().as_dir_dentry_or_err()?;
        let result = if kind == RemoveKind::Rmdir {
            upper_dir.rmdir(&target.name)
        } else {
            upper_dir.unlink(&target.name)
        };
        result.map_err(translate_stale_upper_enoent)?;
        target.inode.mark_name_taken();
        // The removed name is gone from the next merge, so the parent's snapshot is stale.
        **dir_guard = None;
        Ok(())
    }

    /// Publishes a whiteout at a name the upper holds no entry for.
    fn whiteout_lower_only(
        &self,
        target: &RemoveTarget,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper().dentry();
        let published = fs.publish_whiteout(upper_parent, &target.name, None);
        if published.is_ok() {
            target.inode.mark_name_taken();
            // The taken name is gone from the parent's next merge, so its snapshot is stale.
            **dir_guard = None;
        }
        published
    }

    /// Replaces the upper entry behind the name with a whiteout.
    fn replace_upper_with_whiteout(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper().dentry();
        let occupant = target
            .inode
            .upper
            .get()
            .expect("an upper entry replaced by a whiteout has an upper real object");

        let cleared =
            kind == RemoveKind::Rmdir && !read_child_names(occupant.real_inode())?.is_empty();
        if cleared {
            let empty_copy = fs.upper_workdir_inuse().create_workdir_temp(
                &target.name,
                CreateOp::General {
                    kind: InodeType::Dir,
                    mode: occupant.real_inode().mode()?,
                },
            )?;

            let empty_copy_name = String::from(empty_copy.name());

            // The publication displaces this empty copy rather than the occupant: a rename cannot
            // replace a directory with a whiteout, and the displaced directory has to be empty.
            self.exchange_occupant_for_empty_copy(target, occupant, upper_parent, empty_copy)?;

            let workspace = fs.upper_workdir_inuse().workdir_workspace()?;
            let workdir_dir = workspace.as_dir_dentry_or_err()?;
            let cleanup_failure = match workdir_dir.lookup_child(&empty_copy_name) {
                // The displaced directory counts whiteouts, so they are swept before its rmdir.
                Ok(displaced) => {
                    let sweep_failure = fs.sweep_whiteouts(&displaced).err();
                    let rmdir_failure = workdir_dir.rmdir(&empty_copy_name).err();
                    sweep_failure.or(rmdir_failure)
                }
                Err(err) => Some(err),
            };
            if let Some(err) = cleanup_failure {
                warn!(
                    "clear-empty: the displaced-directory cleanup left residue: {:?}",
                    err
                );
            }
        }

        let published = fs
            .publish_whiteout(upper_parent, &target.name, Some(target.inode.type_()))
            .map_err(translate_stale_upper_enoent);
        if cleared || published.is_ok() {
            target.inode.mark_name_taken();
            // The name was taken over here, by the empty copy or by the whiteout, so the parent's
            // snapshot of it is stale.
            **dir_guard = None;
        }
        published
    }

    /// Gives the name an empty copy of the occupant to hold it, the real directory landing in the
    /// workdir, outside the merged view.
    fn exchange_occupant_for_empty_copy(
        &self,
        target: &RemoveTarget,
        occupant: &RealObject,
        upper_parent: &Arc<Dentry>,
        empty_copy: WorkdirTemp,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        // The opaque marker is written before the exchange so the name stays a search barrier.
        if !fs.policy().can_store_private_xattr() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the opaque marker \
                 required for the clear-empty directory exchange",
            ));
        }
        OverlayXattrType::Opaque.set_value_on(empty_copy.dentry(), namespace, None)?;
        // Xattrs are copied before owner/group/mode, so a non-owner rmdir cannot fail `EACCES`.
        self.copy_eligible_xattrs(occupant, &empty_copy, true)?;
        occupant.copy_metadata_to(empty_copy.dentry())?;
        occupant.copy_timestamps_to(empty_copy.dentry())?;
        empty_copy
            .publish(upper_parent, &target.name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        Ok(())
    }
}

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
