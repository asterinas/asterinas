// SPDX-License-Identifier: MPL-2.0

//! The rename recipes: the EXDEV gate ([`OverlayInode::cross_device_gate`])
//! and the upper rename ([`OverlayInode::rename_upper`]).
//!
//! Lock contract: the caller holds both parent directory transaction locks.
//! Permission admission and source promotion are done by the entry before
//! those locks are taken; this module never enters the per-object copy-up
//! coordination lock while holding a parent lock.
//!
//! Notes:
//! - No `RENAME_WHITEOUT`: a source name that still needs a whiteout after
//!   the move is covered by a composed second upper step (rename, then
//!   `publish_whiteout` at the old name); a whiteout target being replaced
//!   inverts via `Exchange`. The VFS interface has no `RENAME_WHITEOUT`, and
//!   both steps run under the same directory transaction domain, so it is
//!   the accepted design rather than a pending TODO.
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

use crate::{
    fs::{
        fs_impls::overlayfs::inode::{Lookup, NegativeLookup, OverlayInode, ReaddirIndex},
        vfs::inode::{Inode, RenameMode},
    },
    prelude::*,
};

pub(super) struct RenameLocks<'a> {
    pub(super) self_index: &'a mut Option<ReaddirIndex>,
    pub(super) target_index: Option<&'a mut Option<ReaddirIndex>>,
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

    /// Any failure after the physical rename triggers a conservative
    /// reconcile of the whole affected set before the error returns.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn rename_upper(
        &self,
        old_name: &str,
        source_inode: &Arc<OverlayInode>,
        target: &Arc<OverlayInode>,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
        mut locks: RenameLocks<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc()?;

        // The VFS-provided source dentry's inode is only a pre-lock
        // admission hint: under the parent locks the source is
        // re-resolved, and any divergence means the rename source went
        // stale.
        let fresh_source = fs.lookup(self, old_name)?;
        match &fresh_source {
            Lookup::Positive(fresh) => {
                if !Arc::ptr_eq(fresh, source_inode) {
                    return Err(Error::new(Errno::ESTALE));
                }
            }
            Lookup::Negative(_) => {
                return Err(Error::new(Errno::ESTALE));
            }
        }

        let source_has_lower = !source_inode.lowers.is_empty();

        // A VFS-provided replaced inode is the source of truth for a
        // positive target, so no fresh target scan is needed; with `None`,
        // the overlay lookup still classifies whiteout/negative targets
        // against merged layer truth.
        let target_lookup = if replaced_inode.is_none() {
            Some(fs.lookup(target, new_name)?)
        } else {
            None
        };
        let target_is_whiteout = replaced_inode.is_none()
            && matches!(
                &target_lookup,
                Some(Lookup::Negative(NegativeLookup::HiddenByWhiteout))
            );
        let target_is_positive =
            replaced_inode.is_some() || matches!(&target_lookup, Some(Lookup::Positive(_)));

        // A visible target under `NoReplace` is `EEXIST`: the upper rename's
        // `NOREPLACE` only observes the upper namespace, so a lower-visible
        // name must still fail.
        if mode == RenameMode::NoReplace && target_is_positive {
            return Err(Error::with_message(
                Errno::EEXIST,
                "the rename target already exists and is visible",
            ));
        }

        // `Replace` over a visible lower-backed directory target requires
        // overlay-visible emptiness (whiteout-hidden children do not count;
        // a pure-upper target defers to the upper rename's own enforcement),
        // and the recorded fresh facts feed the target's whiteout-residue
        // sweep.
        let gate_target_facts = if mode == RenameMode::Replace && target_is_positive {
            let target_object = match replaced_inode {
                Some(replaced) => {
                    Arc::downcast::<OverlayInode>(replaced.clone()).map_err(|_| {
                        Error::with_message(
                            Errno::EIO,
                            "the rename replaced inode is not an overlay inode",
                        )
                    })?
                }
                None => match &target_lookup {
                    Some(Lookup::Positive(target_object)) => target_object.clone(),
                    _ => unreachable!("a positive rename target always has a target object"),
                },
            };
            if target_object.type_().is_directory() {
                let target_facts = target_object.real_object_stack();
                if !target_facts.lowers.is_empty() && target_object.visible_child_count()? != 0 {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay rename target directory is not empty",
                    ));
                }
                Some(target_facts)
            } else {
                None
            }
        } else {
            None
        };

        let upper_parent_path = self.upper_parent_path()?;
        let target_upper_parent_path = target.upper_parent_path()?;

        // Strict and pre-commit: the sweep runs before the physical rename.
        // Unlike the remove recipe's sweep, there is no workdir temp here,
        // so the recipe's pre-commit temp-cleanup step is a no-op.
        if let Some(target_upper_dir) = gate_target_facts
            .as_ref()
            .and_then(|target_facts| target_facts.upper.as_ref())
        {
            super::whiteout::cleanup_upper_whiteouts(
                &fs.real_object_path(target_upper_dir),
                fs.policy().xattr_prefix(),
            )?;
        }

        // A cross-directory move of a source with lower fallback makes the
        // target parent impure: persist the impure marker before the
        // physical rename (before committing the rename).
        let same_parent = self.key(&fs) == target.key(&fs);
        if !same_parent && source_has_lower {
            OverlayInode::set_impure_marker(
                target_upper_parent_path.inode(),
                target_upper_parent_path.dentry(),
                fs.policy().xattr_prefix(),
            )?;
        }

        let mut committed = false;
        let result: Result<()> = (|| {
            // A whiteout target is always replaced or switched, never a
            // visible `NOREPLACE` failure. A lower-backed source switches
            // via `Exchange` so the whiteout lands at the source name
            // without a composed second step; any other source consumes it
            // with `Replace`, and a caller-requested `Exchange` is
            // preserved.
            let effective_mode = match mode {
                RenameMode::Exchange => RenameMode::Exchange,
                _ if target_is_whiteout && source_has_lower => RenameMode::Exchange,
                _ if target_is_whiteout => RenameMode::Replace,
                _ => mode,
            };
            if same_parent {
                upper_parent_path.rename(old_name, &upper_parent_path, new_name, effective_mode)?;
            } else {
                upper_parent_path.rename(
                    old_name,
                    &target_upper_parent_path,
                    new_name,
                    effective_mode,
                )?;
            }
            committed = true;
            if source_has_lower && !target_is_whiteout && mode != RenameMode::Exchange {
                fs.publish_whiteout(&upper_parent_path, old_name, None)?;
            }
            self.finish_whiteout_index(None, locks.self_index);
            if !same_parent {
                let Some(index) = locks.target_index.as_mut() else {
                    unreachable!("a different rename target parent has a lock");
                };
                target.invalidate_readdir_index(index);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(err) => {
                if committed {
                    if same_parent {
                        target.invalidate_readdir_index(locks.self_index);
                        self.invalidate_readdir_index(locks.self_index);
                    } else {
                        let Some(index) = locks.target_index.as_mut() else {
                            unreachable!("a different rename target parent has a lock");
                        };
                        target.invalidate_readdir_index(index);
                        self.invalidate_readdir_index(locks.self_index);
                    }
                }
                return Err(err);
            }
        }
        if !same_parent {
            self.refresh_impure_marker_best_effort(locks.self_index, "rename: source parent");
            let Some(index) = locks.target_index.as_mut() else {
                unreachable!("a different rename target parent has a lock");
            };
            target.refresh_impure_marker_best_effort(index, "rename: target parent");
        }
        Ok(())
    }
}
