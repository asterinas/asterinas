// SPDX-License-Identifier: MPL-2.0

//! The copy-up authority: per-object coordination, winner/waiter trigger,
//! and object-kind promotion.
//!
//! Key concepts:
//! - **copy-up chain**: the lower-only objects from the operation's target up
//!   to the first upper-backed ancestor, collected from the overlay dentry
//!   parent chain with no overlay lock held. That ancestor is the
//!   **publication parent**: it publishes the object below it instead of being
//!   promoted, and each round's promoted object becomes the next round's
//!   publication parent. Every collected element carries the name it publishes
//!   under its publication parent.
//! - **winner/waiter**: the tasks racing on one object's per-inode transaction
//!   lock — which carries no payload for non-directories, because the sole
//!   published-state fact is `upper: Once` being set. The winner performs the
//!   promotion; the waiters re-observe `upper` under that lock and skip.
//! - **promotion round**: one step promoting one object, from the `upper`
//!   re-check through commit, with the publication parent's lock taken before
//!   the object's and both released at the round's end. Under both locks it
//!   reads the round's two name-taken latches before it stages anything.
//!
//! [`OverlayFs::copy_up_at`] is the promotion entry: ancestors promote before
//! the child, and winners serialize through the object's transaction lock.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `workdir` | private workdir temp create/publish/cleanup lifecycle |
//!
//! ## Locking
//!
//! A promotion round reaches the `InodeCache` once: its commit tail rekeys the
//! promoted instance onto the key its identity now resolves to, under the cache
//! guard alone, and the round otherwise never calls back into projection or
//! copy-up — its own latch reads answer whether its coordinate still denotes it.
//!
//! A copy-up holds no overlay lock while it collects its chain. Each promotion
//! round then nests in this order: the publication parent's transaction lock,
//! then the promoted object's transaction lock; below them the only
//! overlay-internal lock a round takes is the commit tail's cache guard,
//! because staging and publication are real-layer calls.
//!
//! - Chain collection acquires no overlay mutex: reading the dentry's parent
//!   and name touches only the VFS dcache (NameAndParent leaf reads). No
//!   child-to-parent edge exists: every round takes the ancestor's lock before
//!   the descendant's, the same direction as the removal and rename
//!   target-object locks.
//! - Both name-taken latches are read under the round's two transaction locks
//!   and before anything is staged, so a name taken away aborts the round
//!   without leaving a workdir temp behind.
//! - The winner holds the promoted object's transaction lock continuously from
//!   the `upper` re-check through the commit tail, and the publication parent's
//!   lock across that same span; the staging and publication calls it makes
//!   there are real-layer calls; the only overlay lock below them is the cache
//!   guard the commit tail's rekey takes.
//! - No lock is released and reacquired inside a round, and no lock domain is
//!   introduced: the per-inode transaction lock and the `InodeCache` guard
//!   remain the only overlay-internal locks.
//!
//! ## References
//!
//! - Linux `ovl_real_file_path` follow-copy-up:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/file.c#L128-L171>
//! - Linux `ovl_set_attr` (symlink mode skip):
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/copy_up.c#L392-L416>

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

    /// One level of a copy-up chain, holding the publication parent's lock then the object's.
    fn promote_level(
        &self,
        publication_parent: &Arc<OverlayInode>,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
    ) -> Result<()> {
        // The publication parent's lock precedes the object's: never a child-to-parent edge.
        let _parent_guard = publication_parent.lock();
        let _object_guard = promoted_object.lock();

        // Another thread may have promoted this object while the chain was being collected.
        if promoted_object.upper.get().is_some() {
            return Ok(());
        }

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

        let (staged, recorded) = promoted_object.stage_in_workdir(name)?;
        if let Err(err) = self.commit_copyup_publication(
            promoted_object,
            name,
            upper_parent.dentry(),
            &staged,
            recorded,
        ) {
            // Best-effort: the rename's own error must survive a cleanup failure.
            let kind = staged.inode().type_();
            let _ = self
                .upper_workdir_inuse()
                .cleanup_workdir_temp(staged.name(), kind);
            return Err(err);
        }
        Ok(())
    }

    /// Commits one staged temp as the published copy-up object.
    fn commit_copyup_publication(
        &self,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        staged: &WorkdirTemp,
        recorded: bool,
    ) -> Result<()> {
        // Holds a strong self-reference so `this` stays upgradeable.
        let _committer = promoted_object.self_arc();
        // The impure marker is best-effort: a failure leaves a missing mirror, not an abort.
        let namespace = self.policy().xattr_namespace();
        if let Ok(false) = OverlayXattrType::Impure.is_positive_on(upper_parent, namespace) {
            let _ = OverlayXattrType::Impure.set_value_on(upper_parent, namespace, None);
        }
        // `publish_workdir_temp` answers the upper dentry the real object is built from.
        let published = self.upper_workdir_inuse().publish_workdir_temp(
            staged,
            upper_parent,
            name,
            RenameMode::Replace,
        )?;
        let published_upper = RealObject::new_upper(published);
        // Without a durable record the identity is the upper's own pair, so the entry keys on it.
        let upper_key = self.identity().real_id_of(&published_upper);
        promoted_object.upper.call_once(|| published_upper);
        // A record the round did not land leaves the instance's identity on its own upper.
        if !recorded
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
    /// Creates this object's staged temp and fills it from the object's real source.
    fn stage_in_workdir(&self, name: &str) -> Result<(WorkdirTemp, bool)> {
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
        let recorded = match self.stage_temp_contents(&temp, source) {
            Ok(recorded) => recorded,
            Err(err) => {
                let _ = upper_workdir.cleanup_workdir_temp(temp.name(), temp.inode().type_());
                return Err(err);
            }
        };
        Ok((temp, recorded))
    }

    /// Fills one staged temp from `source` and reports whether the origin-record step landed; `source`
    /// is the source-side object, while the temp and its dentry are the destination of the copy.
    fn stage_temp_contents(&self, temp: &WorkdirTemp, source: &RealObject) -> Result<bool> {
        source.copy_metadata_to(temp.dentry())?;

        self.copy_eligible_xattrs(source.real_inode(), temp, false)?;

        if temp.inode().type_().is_regular_file() {
            source.copy_data_to(temp.dentry())?;
        }

        // A hard-linked name keys two names alike; a denying mount cannot hold one.
        let recorded = if source.real_inode().type_().is_directory()
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

        Ok(recorded)
    }
}
