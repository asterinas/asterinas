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
        fs_impls::overlayfs::{
            fs::{OverlayFs, mount::WhiteoutCapability},
            inode::{Lookup, NegativeLookup, OverlayInode, ReaddirCache},
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

/// The two directory-transaction guards of one rename, acquired in `Arc::as_ptr` address order.
pub(super) struct RenameLocks<'a, 'b> {
    old_guard: MutexGuard<'a, Option<ReaddirCache>>,
    new_guard: Option<MutexGuard<'b, Option<ReaddirCache>>>,
}

impl<'a, 'b> RenameLocks<'a, 'b> {
    /// Acquires the source and target parent guards in `Arc::as_ptr` address order, once each.
    pub(super) fn acquire(old: &'a OverlayInode, new: &'b Arc<OverlayInode>) -> Result<Self> {
        let old_addr = core::ptr::from_ref(old);
        let new_addr = Arc::as_ptr(new);
        if core::ptr::addr_eq(old_addr, new_addr) {
            let old_guard = old.lock();
            return Ok(Self {
                old_guard,
                new_guard: None,
            });
        }
        if old_addr < new_addr {
            let old_guard = old.lock();
            let new_guard = new.lock();
            Ok(Self {
                old_guard,
                new_guard: Some(new_guard),
            })
        } else {
            let new_guard = new.lock();
            let old_guard = old.lock();
            Ok(Self {
                old_guard,
                new_guard: Some(new_guard),
            })
        }
    }

    /// One disjoint borrow of both payloads.
    pub(super) fn indices(
        &mut self,
    ) -> (&mut Option<ReaddirCache>, Option<&mut Option<ReaddirCache>>) {
        (&mut *self.old_guard, self.new_guard.as_deref_mut())
    }
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

    /// Any failure after the physical rename triggers a conservative reconcile before returning.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn rename_upper(
        &self,
        old_name: &str,
        source_inode: &Arc<OverlayInode>,
        target: &Arc<OverlayInode>,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
        mut locks: RenameLocks<'_, '_>,
    ) -> Result<()> {
        let fs = self.fs_arc();

        // The VFS source inode is only a pre-lock hint; under the locks the source is re-resolved.
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

        // A positive target inode is authoritative; otherwise the overlay lookup classifies it.
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

        // A visible target under `NoReplace` is `EEXIST`: the upper rename only sees upper entries.
        if mode == RenameMode::NoReplace && target_is_positive {
            return Err(Error::with_message(
                Errno::EEXIST,
                "the rename target already exists and is visible",
            ));
        }

        // Replacing a visible lower-backed directory requires overlay-visible emptiness.
        let gate_target: Option<Arc<OverlayInode>> = if mode == RenameMode::Replace
            && target_is_positive
        {
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
                    // Unreachable while `target_is_positive` holds; fail closed with `EIO`.
                    _ => {
                        return Err(Error::with_message(
                            Errno::EIO,
                            "a positive rename target has no target object",
                        ));
                    }
                },
            };
            if target_object.type_().is_directory() {
                if !target_object.lowers.is_empty() && target_object.visible_child_count()? != 0 {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay rename target directory is not empty",
                    ));
                }
                Some(target_object)
            } else {
                None
            }
        } else {
            None
        };

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

        let upper_parent = self.upper_parent_dentry()?;
        let target_upper_parent = target.upper_parent_dentry()?;

        // Strict and pre-commit: the sweep runs before the physical rename.
        if let Some(target_object) = gate_target.as_ref()
            && let Some(target_upper_dir) = target_object.upper.get()
        {
            super::whiteout::cleanup_upper_whiteouts(
                target_upper_dir.dentry(),
                fs.policy().xattr_namespace(),
            )?;
        }

        // Lower-backed cross-directory moves make the parent impure; persist the marker first.
        let same_parent = self.key(&fs) == target.key(&fs);
        if !same_parent && source_has_lower {
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a rename",
                ));
            }
            OverlayInode::set_impure_marker(
                target_upper_parent.inode(),
                target_upper_parent,
                fs.policy().xattr_namespace(),
            )?;
        }

        let mut committed = false;
        let result = self.commit_rename_upper(
            &fs,
            target,
            upper_parent,
            target_upper_parent,
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
                    if same_parent {
                        let (source_index, _) = locks.indices();
                        target.invalidate_readdir_cache(source_index);
                        self.invalidate_readdir_cache(source_index);
                    } else {
                        let (source_index, target_index) = locks.indices();
                        let Some(target_index) = target_index else {
                            return Err(Error::with_message(
                                Errno::EIO,
                                "a different rename target parent has a lock",
                            ));
                        };
                        target.invalidate_readdir_cache(target_index);
                        self.invalidate_readdir_cache(source_index);
                    }
                }
                return Err(err);
            }
        }
        if !same_parent {
            let (source_index, target_index) = locks.indices();
            self.clear_impure_marker(source_index, "rename: source parent");
            let Some(target_index) = target_index else {
                return Err(Error::with_message(
                    Errno::EIO,
                    "a different rename target parent has a lock",
                ));
            };
            target.clear_impure_marker(target_index, "rename: target parent");
        }
        Ok(())
    }

    /// Performs the physical rename and post-rename bookkeeping.
    #[expect(clippy::too_many_arguments)]
    fn commit_rename_upper(
        &self,
        fs: &Arc<OverlayFs>,
        target: &Arc<OverlayInode>,
        upper_parent: &Arc<Dentry>,
        target_upper_parent: &Arc<Dentry>,
        old_name: &str,
        new_name: &str,
        mode: RenameMode,
        target_is_whiteout: bool,
        source_has_lower: bool,
        same_parent: bool,
        locks: &mut RenameLocks<'_, '_>,
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
        let (source_index, target_index) = locks.indices();
        self.finish_whiteout_cache(None, source_index);
        if !same_parent {
            let Some(target_index) = target_index else {
                return Err(Error::with_message(
                    Errno::EIO,
                    "a different rename target parent has a lock",
                ));
            };
            target.invalidate_readdir_cache(target_index);
        }
        Ok(())
    }
}
