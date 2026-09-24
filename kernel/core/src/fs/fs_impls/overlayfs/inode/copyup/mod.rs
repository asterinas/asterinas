// SPDX-License-Identifier: MPL-2.0

//! The copy-up authority: per-object coordination, winner/waiter trigger,
//! and object-kind promotion.
//!
//! Key concepts:
//! - **copy-up chain**: the lower-only objects from the operation's target up
//!   to the first upper-backed ancestor, collected from the overlay dentry
//!   parent chain. That ancestor is the **publication parent**: it publishes
//!   the object below it instead of being promoted, and each round's promoted
//!   object becomes the next round's publication parent. Every collected
//!   element carries the name it publishes under its publication parent.
//! - **winner/waiter**: the tasks racing on one object's per-inode transaction
//!   lock — for a non-directory that lock is a plain serialization token,
//!   because the sole published-state fact is `upper: Once` being set. The
//!   winner performs the promotion; the waiters re-observe `upper` under that
//!   lock and skip.
//! - **promotion round**: one step promoting one object, from the `upper`
//!   re-check through commit.
//!
//! [`OverlayFs::copy_up_at`] is the promotion entry that drives these rounds.
//!
//! # Module map
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`workdir`] | the workdir temp create, publish, and cleanup lifecycle |
//!
//! # Locking
//!
//! A copy-up collects its chain from the VFS dcache alone — reading the dentry's
//! parent and name are NameAndParent leaf reads. Each promotion round then nests in
//! this order — the publication parent's transaction lock, then the promoted
//! object's transaction lock — and releases both at the round's end, so every lock
//! edge runs ancestor-to-descendant.
//!
//! Both name-taken latches of the round are read under those two locks and before
//! anything is staged, so a name taken away aborts the round with nothing staged.
//! The winner holds the promoted object's lock continuously from the `upper`
//! re-check through the commit tail, and the publication parent's lock across that
//! same span.
//!
//! Below the two transaction locks a round takes the commit tail's cache guard,
//! because the staging and publication calls are real-layer calls. That guard is
//! the round's single `InodeCache` touch: the commit tail rekeys the promoted
//! instance onto the key its identity now resolves to, and the round's own latch
//! reads answer whether its coordinate still denotes it.

use self::workdir::WorkdirTemp;
use crate::{
    fs::{
        file::{InodeType, SyncMode},
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{CreateOp, OverlayInode, OverlayXattrType},
            real::RealObject,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode, SymbolicLink},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) mod workdir;

impl OverlayFs {
    /// Promotes every lower-only ancestor above `dentry`, then the object behind it.
    pub(super) fn copy_up_at(&self, dentry: &Dentry) -> Result<()> {
        let promoted = Arc::downcast::<OverlayInode>(dentry.inode().clone())
            .expect("the copy-up operation dentry is not an overlay inode");

        // The dentry chain is walked without an overlay lock; the name is bound here as well.
        let mut chain: Vec<(Arc<OverlayInode>, String)> = Vec::new();
        let mut object = promoted;
        let mut name = dentry.name();
        let mut parent_dentry = dentry.parent();
        let mut publication_parent = loop {
            let Some(parent) = parent_dentry else {
                return Err(Error::with_message(
                    Errno::EIO,
                    "the copy-up operation dentry has no overlay parent",
                ));
            };
            let parent_object = Arc::downcast::<OverlayInode>(parent.inode().clone())
                .expect("the copy-up publication parent is not an overlay inode");
            chain.push((object, name));
            // The first upper-backed ancestor is the boundary: it publishes, it is not promoted.
            if parent_object.upper.get().is_some() {
                break parent_object;
            }
            object = parent_object;
            name = parent.name();
            parent_dentry = parent.parent();
        };

        // Top-down: each round publishes into the object promoted by the round above it.
        for (promoted_object, name) in chain.iter().rev() {
            self.promote_level(&publication_parent, promoted_object, name)?;
            publication_parent = Arc::clone(promoted_object);
        }
        Ok(())
    }

    /// Promotes a rename's target in one round under the publication parent the caller has already
    /// promoted; this entry is the target's own take point and carries the read-only gate.
    pub(super) fn promote_rename_target(
        &self,
        publication_parent: &Arc<OverlayInode>,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
    ) -> Result<()> {
        if self.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }
        self.promote_level(publication_parent, promoted_object, name)
    }

    /// Promotes one level of a copy-up chain under the publication parent's lock, then this object's.
    fn promote_level(
        &self,
        publication_parent: &Arc<OverlayInode>,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
    ) -> Result<()> {
        // The publication parent's lock precedes the object's: never a child-to-parent edge.
        let _parent_guard = publication_parent.lock();
        let _object_guard = promoted_object.lock();

        // Both latches are read under both locks; a set latch fails the round before it stages.
        if promoted_object.is_name_taken() {
            return Err(Error::with_message(
                Errno::ENOENT,
                "the copy-up target name was taken away before the promotion round",
            ));
        }
        if publication_parent.is_name_taken() {
            return Err(Error::with_message(
                Errno::ENOENT,
                "the copy-up publication parent is no longer reachable",
            ));
        }

        // Resolved before staging so a failed probe can never leave a workdir temp behind.
        let upper_parent = publication_parent.writable_upper();

        // The upper entry at this name decides the round: our own published object is a no-op, and
        // any other entry is a name this round must not publish over.
        let upper_dir = upper_parent.dentry().as_dir_dentry_or_err()?;
        let existing = match upper_dir.lookup_child(name) {
            Ok(child) => Some(child),
            Err(err) if err.error() == Errno::ENOENT => None,
            Err(err) => return Err(err),
        };
        match (&existing, promoted_object.upper.get()) {
            (Some(child), Some(ours)) if Arc::ptr_eq(child.inode(), ours.real_inode()) => {
                return Ok(());
            }
            (None, Some(_)) => return Ok(()),
            (Some(_), _) => {
                return Err(Error::with_message(
                    Errno::ESTALE,
                    "the promotion name is already held in the upper",
                ));
            }
            (None, None) => {}
        }

        let source = promoted_object.real_object();
        let staged = promoted_object.stage_in_workdir(name)?;
        // An early `?` here drops the temp unconsumed, so its guard removes it from the workspace.
        let origin_record_landed = promoted_object.stage_temp_contents(&staged, source)?;
        self.commit_copyup_publication(
            promoted_object,
            name,
            upper_parent.dentry(),
            staged,
            origin_record_landed,
        )?;
        Ok(())
    }

    /// Commits one staged temp as the published copy-up object.
    fn commit_copyup_publication(
        &self,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        staged: WorkdirTemp,
        origin_record_landed: bool,
    ) -> Result<()> {
        // The impure marker is best-effort: a failure leaves a missing mirror, not an abort.
        let namespace = self.policy().xattr_namespace();
        if let Ok(false) = OverlayXattrType::Impure.is_positive_on(upper_parent, namespace) {
            let _ = OverlayXattrType::Impure.set_value_on(upper_parent, namespace, None);
        }
        let published = staged.dentry().clone();
        staged.publish(upper_parent, name, RenameMode::Replace)?;
        let published_upper = RealObject::new_upper(published);
        // Without a durable record the identity is the upper's own pair, so the entry keys on it.
        let upper_key = self.identity().real_id_of(&published_upper);
        promoted_object.upper.call_once(|| published_upper);
        // A record the round did not land leaves the instance's identity on its own upper.
        if !origin_record_landed
            && !promoted_object.type_().is_directory()
            && let Some(lower) = promoted_object.lowers.first()
        {
            self.inodes().rekey(
                self.identity().real_id_of(lower),
                upper_key,
                promoted_object,
            );
        }
        Ok(())
    }
}

impl OverlayInode {
    /// Creates this object's staged temp: an empty real object in the workdir that mirrors this
    /// object's kind, its mode, and, for a symlink, its target.
    fn stage_in_workdir(&self, name: &str) -> Result<WorkdirTemp> {
        let fs = self.fs_arc();
        let upper_workdir = fs.upper_workdir_inuse();
        let source = self.real_object();
        let temp = match source.real_inode().type_() {
            InodeType::Dir => {
                // Atomic `Replace` replaces a stale upper entry instead of failing with `EEXIST`.
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::General {
                        kind: InodeType::Dir,
                        mode,
                    },
                )?
            }
            InodeType::File => {
                // Data is synced before the atomic rename, so the published object is durable.
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::General {
                        kind: InodeType::File,
                        mode,
                    },
                )?
            }
            InodeType::SymLink => {
                // The symlink is created with its target; the target object itself is not copied.
                let mode = source.real_inode().mode()?;
                let target = match source.real_inode().read_link()? {
                    SymbolicLink::Plain(target) => target,
                    SymbolicLink::Path(_) => {
                        return_errno_with_message!(
                            Errno::EOPNOTSUPP,
                            "a path-style symlink target cannot be copied up"
                        );
                    }
                };
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::Symlink {
                        target: &target,
                        mode,
                    },
                )?
            }
            InodeType::CharDevice
            | InodeType::BlockDevice
            | InodeType::NamedPipe
            | InodeType::Socket => {
                let mknod_type = match source.real_inode().type_() {
                    InodeType::NamedPipe => MknodType::NamedPipe,
                    InodeType::CharDevice => {
                        let rdev = source
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower char device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::CharDevice(rdev)
                    }
                    InodeType::BlockDevice => {
                        let rdev = source
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower block device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::BlockDevice(rdev)
                    }
                    _ => {
                        return Err(Error::with_message(
                            Errno::EOPNOTSUPP,
                            "socket nodes cannot be copied up",
                        ));
                    }
                };
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::Mknod {
                        mode,
                        node: &mknod_type,
                    },
                )?
            }
            InodeType::Unknown => {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "cannot promote an overlay object of unknown type",
                ));
            }
        };
        Ok(temp)
    }

    /// Fills one staged temp from `source` and reports whether the origin-record step landed; `source`
    /// is the source-side object, while the temp and its dentry are the destination of the copy.
    fn stage_temp_contents(&self, temp: &WorkdirTemp, source: &RealObject) -> Result<bool> {
        source.copy_metadata_to(temp.dentry())?;

        self.copy_eligible_xattrs(source, temp, false)?;

        if temp.inode().type_().is_regular_file() {
            source.copy_data_to(temp.dentry())?;
        }

        // A hard-linked name keys two names alike; a denying mount cannot hold one.
        let origin_record_landed = if source.real_inode().type_().is_directory()
            || source.real_inode().metadata()?.nr_hard_links <= 1
        {
            self.fs_arc().record_origin(temp, source)?
        } else {
            false
        };

        source.copy_timestamps_to(temp.dentry())?;

        if temp.inode().type_().is_regular_file() {
            temp.inode().sync(SyncMode::Full)?;
        }

        Ok(origin_record_landed)
    }
}
