// SPDX-License-Identifier: MPL-2.0

//! The rename recipes: the EXDEV gate ([`OverlayInode::cross_device_gate`])
//! and the upper rename ([`OverlayInode::rename_upper`]).
//!
//! Lock contract: the recipe runs under the rename's fixed-arity lock set — at
//! most four members (both parent directories, the source object, and the
//! target object a `Replace` overwrites), classified from the VFS dentries and
//! taken in one pass in ascending key order `K = (depth, address)`; both name
//! bindings are re-validated under that set, and a classification that went
//! stale restarts the attempt within a fixed bound before failing `ESTALE`.
//! The writable take point (the read-only gate plus the copy-up promotion) runs
//! in the entry before those locks are taken, so copy-up is entered with no
//! overlay lock held and takes the publication parent's lock before the promoted
//! object's. The whiteout verdict probes the lower layers of the source parent
//! inside those locks, and it is resolved once for the whole rename. The
//! overwritten target also gets its name-taken latch set, once the physical
//! rename has committed.
//!
//! Notes:
//! - No `RENAME_WHITEOUT`: a source name that still needs a whiteout after
//!   the move is covered by a composed second upper step (rename, then
//!   `publish_whiteout` at the old name); a whiteout target being replaced
//!   inverts via `Exchange`. The VFS interface has no `RENAME_WHITEOUT`, and
//!   both steps run under the same directory transaction domain, so it is
//!   the accepted design rather than a pending TODO. The old name needs that
//!   whiteout whenever a lower layer still holds it, which is what the
//!   layer-internal probe decides; the source object's own stack says nothing
//!   about the name once the source has been promoted.
//! - The `redirect_dir` policy is not implemented, so the flat EXDEV default applies.
//! - Target fallback is covered by the moved source: after a successful move,
//!   the target name is backed by the moved source's own upper/lower state,
//!   so no separate target projection or whiteout is needed.
//! - Overlay does not maintain merged `nlink` accounting: `metadata()` reports
//!   the visible source real inode's link count, so lower-layer additional
//!   links are not added into a synthetic overlay count.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/copy_up.c#L1295-L1297>
//!   (Linux `ovl_copy_up` pre-rename copy-up)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L1080-L1308>
//!   (Linux `ovl_rename` replace gate)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L361-L430>
//!   (Linux `ovl_clear_empty` whiteout-residue sweep)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/namei.c#L1418-L1480>
//!   (Linux `ovl_lower_positive` lower-presence check)

use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::{OverlayFs, mount::WhiteoutCapability},
            inode::{
                Lookup, NegativeLookup, OverlayInode, OverlayInodeLockPayload, OverlayXattrType,
            },
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

const ROLE_OLD_PARENT: usize = 0;
const ROLE_NEW_PARENT: usize = 1;
const ROLE_SOURCE: usize = 2;
const ROLE_TARGET: usize = 3;
const RENAME_ROLE_COUNT: usize = 4;
/// One rename's bound on classify/take/validate attempts; exhausting it fails with `ESTALE`.
const RENAME_LOCK_ATTEMPT_LIMIT: usize = 3;

/// Both parents, the source, and a `Replace` target: each distinct object locked once.
pub(super) struct RenameLocks<'x> {
    old_parent_guard: MutexGuard<'x, OverlayInodeLockPayload>,
    new_parent_guard: Option<MutexGuard<'x, OverlayInodeLockPayload>>,
    #[expect(
        dead_code,
        reason = "the guard is held to keep the renamed source locked"
    )]
    source_guard: Option<MutexGuard<'x, OverlayInodeLockPayload>>,
    #[expect(
        dead_code,
        reason = "the guard is held to keep the overwritten target locked"
    )]
    target_guard: Option<MutexGuard<'x, OverlayInodeLockPayload>>,
    /// The role whose guard serves each role: itself, or an earlier role naming the same object.
    owner_roles: [Option<usize>; RENAME_ROLE_COUNT],
    /// The in-lock re-resolved verdict on the target name, reused as the recipe's target answer.
    target: Lookup,
}

impl<'x> RenameLocks<'x> {
    /// Classifies, locks, and re-validates both bindings; `Ok(None)` means it went stale.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn acquire(
        old_parent_dentry: &'x Dentry,
        old_parent: &'x OverlayInode,
        old_name: &str,
        new_parent_dentry: &'x Dentry,
        new_parent: &'x Arc<OverlayInode>,
        source: &'x Arc<OverlayInode>,
        new_name: &str,
        target: Option<&'x Arc<OverlayInode>>,
    ) -> Result<Option<Self>> {
        let fs = old_parent.fs_arc();
        // Roles in ascending order: old parent, new parent, source, and the optional target.
        let members: [Option<&'x OverlayInode>; RENAME_ROLE_COUNT] = [
            Some(old_parent),
            Some(new_parent.as_ref()),
            Some(source.as_ref()),
            target.map(|object| object.as_ref()),
        ];

        // A role naming an already-named object is served by that owner's guard, not relocked.
        let mut owner_roles: [Option<usize>; RENAME_ROLE_COUNT] = [None; RENAME_ROLE_COUNT];
        for role in 0..RENAME_ROLE_COUNT {
            let Some(object) = members[role] else {
                continue;
            };
            let address = core::ptr::from_ref(object);
            owner_roles[role] = Some(
                (0..role)
                    .find(|earlier| {
                        members[*earlier].is_some_and(|member| {
                            core::ptr::addr_eq(core::ptr::from_ref(member), address)
                        })
                    })
                    .unwrap_or(role),
            );
        }

        // Rule `R` in one take pass: an absent member keeps the maximum key and is skipped below.
        let old_parent_depth = dentry_depth(old_parent_dentry);
        let new_parent_depth = dentry_depth(new_parent_dentry);
        let mut ordered = [
            (usize::MAX, usize::MAX, ROLE_OLD_PARENT),
            (usize::MAX, usize::MAX, ROLE_NEW_PARENT),
            (usize::MAX, usize::MAX, ROLE_SOURCE),
            (usize::MAX, usize::MAX, ROLE_TARGET),
        ];
        for role in 0..RENAME_ROLE_COUNT {
            let Some(object) = members[role] else {
                continue;
            };
            if owner_roles[role] != Some(role) {
                continue;
            }
            let (depth, address) = member_key(role, old_parent_depth, new_parent_depth, object);
            ordered[role] = (depth, address, role);
        }
        for index in 1..RENAME_ROLE_COUNT {
            let mut position = index;
            while position > 0 && ordered[position - 1] > ordered[position] {
                ordered.swap(position - 1, position);
                position -= 1;
            }
        }

        let mut guards: [Option<MutexGuard<'x, OverlayInodeLockPayload>>; RENAME_ROLE_COUNT] =
            [None, None, None, None];
        for (_, _, role) in ordered {
            let Some(object) = members[role] else {
                continue;
            };
            if owner_roles[role] != Some(role) {
                continue;
            }
            guards[role] = Some(object.lock());
        }
        let old_parent_guard = guards[ROLE_OLD_PARENT].take().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the rename lock set is missing the source parent",
            )
        })?;
        let new_parent_guard = guards[ROLE_NEW_PARENT].take();
        let source_guard = guards[ROLE_SOURCE].take();
        let target_guard = guards[ROLE_TARGET].take();

        // Both bindings are re-resolved under the guards: a rebound name gives the wrong mutex.
        let source_is_fresh = match fs.lookup(old_parent, old_name)? {
            Lookup::Positive(fresh) => Arc::ptr_eq(&fresh, source),
            Lookup::Negative(_) => false,
        };
        let verdict = fs.lookup(new_parent, new_name)?;
        let target_is_fresh = match (target, &verdict) {
            (Some(member), Lookup::Positive(fresh)) => Arc::ptr_eq(member, fresh),
            (None, Lookup::Negative(_)) => true,
            _ => false,
        };
        if !source_is_fresh || !target_is_fresh {
            return Ok(None);
        }

        Ok(Some(Self {
            old_parent_guard,
            new_parent_guard,
            source_guard,
            target_guard,
            owner_roles,
            target: verdict,
        }))
    }

    pub(super) fn parent_lock_payloads(
        &mut self,
    ) -> (
        &mut OverlayInodeLockPayload,
        Option<&mut OverlayInodeLockPayload>,
    ) {
        (
            &mut *self.old_parent_guard,
            self.new_parent_guard.as_deref_mut(),
        )
    }

    fn target_verdict(&self) -> &Lookup {
        &self.target
    }
}

/// The rule-`R` key of one member; a child member is one step below its own operation parent.
fn member_key(
    role: usize,
    old_parent_depth: usize,
    new_parent_depth: usize,
    object: &OverlayInode,
) -> (usize, usize) {
    let depth = match role {
        ROLE_OLD_PARENT => old_parent_depth,
        ROLE_NEW_PARENT => new_parent_depth,
        ROLE_SOURCE => old_parent_depth + 1,
        // The overwritten target is a child of the new parent whether it is visible or not.
        ROLE_TARGET => new_parent_depth + 1,
        _ => return (usize::MAX, usize::MAX),
    };
    (depth, core::ptr::from_ref(object) as usize)
}

/// The number of dcache parent steps from `dentry` up to the mount root, which has no parent.
fn dentry_depth(dentry: &Dentry) -> usize {
    let mut depth = 0;
    let mut current = dentry.parent();
    while let Some(parent) = current {
        depth += 1;
        current = parent.parent();
    }
    depth
}

impl OverlayInode {
    pub(super) fn cross_device_gate(&self, source_inode: &Arc<OverlayInode>) -> Result<()> {
        if !source_inode.type_().is_directory() {
            return Ok(());
        }
        if source_inode.lowers.is_empty() {
            return Ok(());
        }
        Err(Error::with_message(
            Errno::EXDEV,
            "the overlay cross-directory rename of a lower-backed or merged directory \
             requires the not-yet-implemented redirect_dir policy",
        ))
    }

    /// The rename entry: classify, lock in `K` order, re-validate, then run the upper rename.
    pub(super) fn rename_with_batch_locks(
        &self,
        old_child_dentry: &Dentry,
        new_parent_dentry: &Dentry,
        new_name: &str,
        target_dentry: Option<&Dentry>,
        mode: RenameMode,
    ) -> Result<()> {
        let old_name = old_child_dentry.name();
        // VFS names the child through its parent, so `None` is only a fail-closed safety net.
        let old_parent_dentry = old_child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the renamed child has no parent dentry")
        })?;
        let fs = self.fs_arc();
        for _ in 0..RENAME_LOCK_ATTEMPT_LIMIT {
            // Redone per attempt: an instance lives as long as its own attempt's guards.
            let source =
                Arc::downcast::<OverlayInode>(old_child_dentry.inode().clone()).map_err(|_| {
                    Error::with_message(Errno::EIO, "the rename source is not an overlay inode")
                })?;
            let new_parent = Arc::downcast::<OverlayInode>(new_parent_dentry.inode().clone())
                .map_err(|_| {
                    Error::with_message(Errno::EIO, "the rename target is not an overlay inode")
                })?;
            let target = match target_dentry {
                Some(dentry) => Some(
                    Arc::downcast::<OverlayInode>(dentry.inode().clone()).map_err(|_| {
                        Error::with_message(Errno::EIO, "the rename target is not an overlay inode")
                    })?,
                ),
                // A target the VFS did not resolve is classified by one unlocked lookup here.
                None => match fs.lookup(&new_parent, new_name)? {
                    Lookup::Positive(object) => Some(object),
                    Lookup::Negative(_) => None,
                },
            };
            let locks = RenameLocks::acquire(
                &old_parent_dentry,
                self,
                &old_name,
                new_parent_dentry,
                &new_parent,
                &source,
                new_name,
                target.as_ref(),
            )?;
            let Some(locks) = locks else {
                continue;
            };
            return self.rename_upper(&old_name, &source, &new_parent, new_name, mode, locks);
        }
        // Every attempt saw a name rebound under its own guards; fail without any side effect.
        Err(Error::new(Errno::ESTALE))
    }

    /// Any failure after the physical rename triggers a conservative reconcile before returning.
    pub(super) fn rename_upper(
        &self,
        old_name: &str,
        source_inode: &Arc<OverlayInode>,
        target: &Arc<OverlayInode>,
        new_name: &str,
        mode: RenameMode,
        mut locks: RenameLocks<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();

        // The name keeps a lower entry: the probe is the only answer that survives promotion.
        let namespace = fs.policy().xattr_namespace();
        let source_has_lower = self.has_lower_entry(old_name, namespace)?;

        // The in-lock verdict replaces the hints: a positive target is what is overwritten.
        let target_object: Option<Arc<OverlayInode>> = match locks.target_verdict() {
            Lookup::Positive(object) => Some(object.clone()),
            Lookup::Negative(_) => None,
        };
        let target_is_whiteout = matches!(
            locks.target_verdict(),
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        );

        // A visible target under `NoReplace` is `EEXIST`: the upper rename only sees upper entries.
        if mode == RenameMode::NoReplace && target_object.is_some() {
            return Err(Error::with_message(
                Errno::EEXIST,
                "the rename target already exists and is visible",
            ));
        }

        // A `Replace` over a positive target is the one rename that takes a name away.
        let replaced_object = if mode == RenameMode::Replace {
            target_object
        } else {
            None
        };

        // Replacing a visible lower-backed directory needs emptiness under the target's own guard.
        if let Some(object) = replaced_object.as_ref()
            && object.type_().is_directory()
            && !object.lowers.is_empty()
        {
            if locks.owner_roles[ROLE_TARGET].is_none() {
                return Err(Error::with_message(
                    Errno::EIO,
                    "the rename lock set has no guard for the overwritten target",
                ));
            }
            if !object.is_empty_dir()? {
                return Err(Error::with_message(
                    Errno::ENOTEMPTY,
                    "the overlay rename target directory is not empty",
                ));
            }
        }

        // A rename that publishes a whiteout must first verify the upper can represent one.
        if source_has_lower
            && !target_is_whiteout
            && mode != RenameMode::Exchange
            && fs.policy().whiteout_capability() == WhiteoutCapability::Unsupported
        {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the rename \
                 cannot publish one",
            ));
        }

        let upper_parent = self.writable_upper();
        let target_upper_parent = target.writable_upper();

        // Strict and pre-commit: the sweep runs before the physical rename.
        if let Some(target_object) = replaced_object.as_ref()
            && target_object.type_().is_directory()
            && let Some(target_upper_dir) = target_object.upper.get()
        {
            super::whiteout::cleanup_upper_whiteouts(
                target_upper_dir.dentry(),
                fs.policy().xattr_namespace(),
            )?;
        }

        // A cross-directory move of an origin-preserved source makes the parent impure.
        let same_parent = self.same_directory_as(target);
        // The source object retains an upper here: the rename entry promotes the source name
        // before the lock batch is taken, so the identity question below is asked of that upper.
        // A directory source crosses parents through the gate defined in this file, which gives
        // `EXDEV` for a directory that retains a lower, and a same-parent move short-circuits on
        // `!same_parent` right here, so the source this line judges is a file or a lower-free
        // directory.
        // Were that gate lifted, a directory's origin-backed identity would be a "which lower is
        // retained" question rather than a record question, and this line would change with it
        // into a check of the retained lowers.
        let marks_target_impure = !same_parent && {
            let source_upper = source_inode.writable_upper();
            fs.origin_of(source_upper.dentry()).is_some()
        };
        if marks_target_impure {
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a rename",
                ));
            }
            if !OverlayXattrType::Impure.is_positive_on(target_upper_parent.dentry(), namespace)? {
                OverlayXattrType::Impure.set_value_on(
                    target_upper_parent.dentry(),
                    namespace,
                    None,
                )?;
            }
        }

        let mut committed = false;
        let result = self.commit_rename_upper(
            &fs,
            upper_parent.dentry(),
            target_upper_parent.dentry(),
            old_name,
            new_name,
            mode,
            target_is_whiteout,
            source_has_lower,
            same_parent,
            &mut locks,
            &mut committed,
        );
        match result {
            Ok(()) => {}
            Err(err) => {
                if committed {
                    // A partially committed rename already gave the target name to the source.
                    if let Some(object) = replaced_object.as_ref() {
                        object.mark_name_taken();
                    }
                    if same_parent {
                        let (old_parent_lock_payload, _) = locks.parent_lock_payloads();
                        // Both names of the pair belong to the parent's next merge.
                        *old_parent_lock_payload = None;
                    } else {
                        let (old_parent_lock_payload, new_parent_lock_payload) =
                            locks.parent_lock_payloads();
                        let Some(new_parent_lock_payload) = new_parent_lock_payload else {
                            return Err(Error::with_message(
                                Errno::EIO,
                                "a different rename target parent has a lock",
                            ));
                        };
                        // Both parents lost or gained a name, so both snapshots are stale.
                        *new_parent_lock_payload = None;
                        *old_parent_lock_payload = None;
                    }
                }
                return Err(err);
            }
        }
        // The overwritten object lost its name; a later round publishes under a stale coordinate.
        if let Some(object) = replaced_object.as_ref() {
            object.mark_name_taken();
        }
        Ok(())
    }

    /// Performs the physical rename and post-rename bookkeeping.
    #[expect(clippy::too_many_arguments)]
    fn commit_rename_upper(
        &self,
        fs: &Arc<OverlayFs>,
        upper_parent: &Arc<Dentry>,
        target_upper_parent: &Arc<Dentry>,
        old_name: &str,
        new_name: &str,
        mode: RenameMode,
        target_is_whiteout: bool,
        source_has_lower: bool,
        same_parent: bool,
        locks: &mut RenameLocks<'_>,
        committed: &mut bool,
    ) -> Result<()> {
        // A whiteout target is always replaced or switched, never a visible `NOREPLACE` failure.
        let effective_mode = match mode {
            RenameMode::Exchange => RenameMode::Exchange,
            _ if target_is_whiteout && source_has_lower => RenameMode::Exchange,
            _ if target_is_whiteout => RenameMode::Replace,
            _ => mode,
        };
        if same_parent {
            upper_parent.as_dir_dentry_or_err()?.rename(
                old_name,
                &upper_parent.as_dir_dentry_or_err()?,
                new_name,
                effective_mode,
            )?;
        } else {
            upper_parent.as_dir_dentry_or_err()?.rename(
                old_name,
                &target_upper_parent.as_dir_dentry_or_err()?,
                new_name,
                effective_mode,
            )?;
        }
        *committed = true;
        if source_has_lower && !target_is_whiteout && mode != RenameMode::Exchange {
            fs.publish_whiteout(upper_parent, old_name, None)?;
        }
        let (old_parent_lock_payload, new_parent_lock_payload) = locks.parent_lock_payloads();
        // The moved-away name left the source parent's visible set, so its snapshot is stale.
        *old_parent_lock_payload = None;
        if !same_parent {
            let Some(new_parent_lock_payload) = new_parent_lock_payload else {
                return Err(Error::with_message(
                    Errno::EIO,
                    "a different rename target parent has a lock",
                ));
            };
            // The new name entered the target parent's visible set, so its snapshot is stale too.
            *new_parent_lock_payload = None;
        }
        Ok(())
    }
}
