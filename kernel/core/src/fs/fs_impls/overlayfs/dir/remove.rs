// SPDX-License-Identifier: MPL-2.0

//! The remove recipes.
//!
//! This module hosts the single recipe helper on [`OverlayInode`]:
//! [`OverlayInode::remove_target`] — the shared unlink/rmdir recipe
//! parameterized by the closed [`RemoveKind`] vocabulary (the former
//! `is_dir: bool` flag is replaced by [`RemoveKind::{Unlink, Rmdir}`] so call
//! sites name the operation) that, under the caller-held parent `DIR`,
//! re-derives the fresh `(parent, name)` projection (`ENOENT`), runs the
//! overlay-visible emptiness gate (`visible_child_count`) before any upper
//! removal, and then decides **pure-upper direct removal** (upper
//! `unlink`/`rmdir`, no whiteout) versus **lower-backed whiteout publication**
//! (`publish_whiteout` over the removed upper object, plus the clear-empty
//! opaque-temp exchange when the upper directory of a lower-backed directory
//! holds hidden entries that would otherwise leak or resist workdir cleanup).
//! The thin `Inode::unlink`/`Inode::rmdir` entries live in the sibling
//! `dir/mod.rs` and delegate into this file; `visible_child_count` is consumed
//! from the merged-directory module, never re-implemented.
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! Lock contract: the caller (the `dir/mod.rs` entry) holds the parent `DIR`
//! transaction lock and has pinned the mount. This module acquires no Overlay
//! lock of its own beyond the brief `INODE` facts snapshots inside
//! `facts_snapshot`/`select_real_inode` (snapshot-and-release, never held
//! across an underlying call), the brief index `INODE` sections inside the
//! `visible_child_count`/`readdir_index_remove` entries, and the `CUL` domain
//! entered inside the real stage of
//! `check_permission(AccessType::Mutating, ...)` (promotes the parent under
//! the caller-held `DIR`). Upper/workdir physical operations
//! (`unlink`/`rmdir`/`rename`/`set_xattr`) run through the base VFS `Path`
//! layer in the sleep-capable `DIR` domain under the underlying filesystem's
//! own locking; no `WL`/spin domain is entered and no `WL` payload is
//! touched (the whiteout cache and the whiteout publish mechanics are the
//! sibling `dir/whiteout.rs` owner). All `DIR`/`CUL`/`INODE` domains are
//! released before any VFS-visible return; `MOUNT` is never acquired.
//!
//! No `.unwrap()`/`.expect()` appears in any production path; hard invariant
//! failures use the `Error::with_message`/`unreachable!` precedents.

use super::whiteout;
use crate::{
    fs::{
        file::{InodeType, Permission},
        fs_impls::overlayfs::{
            AccessType,
            copyup::WorkdirTempRequest,
            metadata_security::xattr::{
                OPAQUE_MARKER_VALUE, OPAQUE_XATTR_FULL_NAME, XattrCopyPolicy,
            },
            mount::OverlayFs,
            projection::{
                Binding, BindingKey, HiddenEvidence, NegativeBinding, OverlayInode,
                OverlayObjectFacts,
            },
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::{self, Path},
            xattr::{XattrName, XattrSetFlags},
        },
    },
    prelude::*,
};

/// The remove operation kind of [`OverlayInode::remove_target`].
///
/// Closed two-variant set: [`RemoveKind::Unlink`] (type gate `EISDIR` +
/// direct unlink/whiteout publish) and [`RemoveKind::Rmdir`] (emptiness gate
/// plus clear-empty opaque exchange). The `remove_target` recipe branches on
/// this closed vocabulary instead of a boolean.
///
/// TODO(doc): the recipe prose in [`OverlayInode::remove_target`] describes
/// the shared unlink/rmdir structure in one block; a future revision may
/// split per-variant contract notes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RemoveKind {
    /// The unlink operation.
    Unlink,
    /// The rmdir operation.
    Rmdir,
}

impl OverlayInode {
    /// Removes one visible name from this overlay directory.
    ///
    /// The shared unlink/rmdir recipe:
    ///
    /// 1. **Fresh projection:** `lookup_binding` under the caller-held parent
    ///    `DIR` re-derives the current positive/negative binding — never a
    ///    stale VFS dentry. The target must be `Positive`; a negative
    ///    projection (absent or hidden) is `Err(ENOENT)`.
    /// 2. **Visible-emptiness gate (rmdir only):** `visible_child_count`
    ///    counts the overlay-visible children: any visible upper/lower/merged
    ///    child → `ENOTEMPTY`; a non-directory target → `ENOTDIR`; a
    ///    `NeedsRebuild`-unresolvable index → conservative `ENOTEMPTY` (never
    ///    an upper-only emptiness guess). Whiteout-hidden children do not
    ///    count. `unlink` skips this gate.
    /// 3. **Permission admission:** `check_permission(AccessType::Mutating,
    ///    MAY_WRITE)` promotes this parent to upper authority under the
    ///    caller-held `DIR` (the entry's EROFS gate is the other admission
    ///    point), then `upper_parent_path()` resolves the promoted upper
    ///    real parent `Path`.
    /// 4. **Pure-upper target** (upper-backed with no lower fallback and no
    ///    opaque barrier): direct `upper_parent_path.rmdir(name)`/
    ///    `unlink(name)` through the base VFS `Path` layer (no whiteout);
    ///    publication inline: `BindingCache::invalidate` +
    ///    `readdir_index_remove` (both steps infallible, so no reconcile arm
    ///    is reachable on this path). When the fresh projection asserted an
    ///    upper object at `name` and the physical upper unlink/rmdir reports
    ///    `ENOENT`, the recipe returns `ESTALE` (the upper object became
    ///    stale behind the overlay; Linux `ovl_remove_upper` /
    ///    `ovl_remove_and_whiteout`); every other upper error propagates
    ///    unchanged.
    /// 5. **Upper-over-lower / lower-only / opaque-over-lower target:**
    ///    publication of a whiteout at `(upper_parent_path, name)` via the
    ///    sibling `publish_whiteout` helper (`Replace` over a present
    ///    non-dir upper object, `Exchange` + workdir cleanup of the
    ///    displaced dir for a present upper directory, `link` for an absent
    ///    upper name); for a lower-backed **directory** whose upper dir
    ///    holds entries (necessarily whiteouts — the emptiness gate has
    ///    already refused visible children), the clear-empty path
    ///    first replaces the upper dir with a workdir-prepared
    ///    opaque temp dir (atomic `Exchange`), cleans the displaced old
    ///    upper dir in the workdir, and then lets `publish_whiteout`
    ///    exchange the whiteout over the opaque temp. Publication inline:
    ///    `BindingCache::insert` `Negative(HiddenByWhiteout(HiddenEvidence))`
    ///    — the `HiddenEvidence` pin re-observes the published whiteout from
    ///    the upper — + `readdir_index_remove`. The recipe distinguishes the
    ///    pre-publication failure arm (best-effort workdir temp cleanup;
    ///    lower authority stays valid) from the post-physical-success arm
    ///    (conservative reconcile), honoring the never-partial contract. A
    ///    physical-upper `ENOENT` on the asserted-upper arms (the clear-empty
    ///    exchange or the whiteout `Replace`/`Exchange` publish) is translated
    ///    to `ESTALE`; the `None` link arm keeps `ENOENT` unchanged (the
    ///    projection asserted no upper object).
    ///
    /// # Notes
    ///
    /// - A directory target is refused with `Err(EISDIR)` on the `unlink`
    ///   entry: the Asterinas VFS routes `unlink` on a directory into the fs,
    ///   so without this gate a lower-backed directory would be whiteout-
    ///   hidden instead of refused.
    /// - An opaque upper directory is classified as lower-backed (its
    ///   `facts.lowers()` is empty by the opaque barrier rule, but a
    ///   hidden lower counterpart exists — Linux `ovl_lower_positive`); the
    ///   `is_opaque_directory()` probe extends the pure-upper test so rmdir
    ///   publishes a whiteout instead of exposing the hidden lower directory.
    pub(super) fn remove_target(&self, name: &str, kind: RemoveKind) -> Result<()> {
        // The mount is pinned by the entry; the parent `DIR` is held.
        let fs = self.fs_arc()?;
        // Step 1 — fresh projection under `DIR`: the VFS dentry may be stale,
        // the `DIR`-domain projection is authoritative. A negative projection
        // (absent or hidden) is `ENOENT`; the target must be visible.
        let parent_facts = self.facts_snapshot();
        let lookup = fs.lookup_binding(&parent_facts, name)?;
        // Stale-upper routing (step 1): a previously published upper-backed
        // positive binding whose physical upper object vanished behind the
        // overlay with no whiteout left surfaces `ESTALE` (Linux
        // `ovl_remove_and_whiteout`) before the rmdir emptiness gate and the
        // unlink `EISDIR` type gate below — the fresh projection already
        // established that the asserted upper object is gone.
        if lookup.is_stale_upper {
            return Err(translate_stale_upper_enoent(Error::with_message(
                Errno::ENOENT,
                "the overlay target became stale behind the overlay",
            )));
        }
        let target_inode = lookup.binding.into_inode().ok_or_else(|| {
            Error::with_message(Errno::ENOENT, "the overlay target does not exist")
        })?;
        let target_facts = target_inode.facts_snapshot();

        if kind == RemoveKind::Rmdir {
            // Step 2 — the visible-emptiness gate (runs before any upper
            // removal). The index integration point ensures the target index is `Valid`
            // (rebuild under the same `DIR` transaction) and counts the
            // `Visible` entries; `.`/`..` are never entries and
            // whiteout-hidden children do not count.
            match target_inode.visible_child_count(&target_facts) {
                Ok(0) => {}
                Ok(_) => {
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory is not empty",
                    ));
                }
                Err(err) if err.error() == Errno::ENOTDIR => {
                    // A non-directory target cannot be rmdir'd.
                    return Err(err);
                }
                Err(_) => {
                    // `NeedsRebuild`-unresolvable: conservative `ENOTEMPTY`
                    // (never an upper-only emptiness guess).
                    return Err(Error::with_message(
                        Errno::ENOTEMPTY,
                        "the overlay directory emptiness could not be verified",
                    ));
                }
            }
        } else if target_inode.type_().is_directory() {
            // Defensive type gate (see the method doc): the Asterinas VFS
            // routes `unlink` on a directory into the fs without refusing it,
            // so every fs gates itself (ramfs precedent). Without this gate a
            // lower-backed directory would be whiteout-hidden instead of
            // refused with `EISDIR`.
            return Err(Error::with_message(
                Errno::EISDIR,
                "a directory cannot be unlinked",
            ));
        }

        // Pure-upper vs lower-backed classification. An upper object with an
        // empty lower stack is pure-upper ONLY when it is not an opaque
        // directory: an opaque upper directory is a lower-search barrier
        // whose hidden lower counterpart still exists (Linux
        // `ovl_lower_positive`), so removing it must publish a whiteout
        // rather than expose the lower.
        let is_pure_upper = match target_facts.upper() {
            Some(upper_obj) => {
                target_facts.lowers().is_empty() && !upper_obj.is_opaque_directory()?
            }
            None => false,
        };

        // Step 3/4 — the recipe's own permission admission (promotes the
        // parent under the held `DIR`; the entry's EROFS gate is the other
        // admission point) and the promoted upper real parent `Path` (the
        // physical-operation target).
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        let upper_parent_path = self.upper_parent_path()?;

        if is_pure_upper {
            // A pure-upper rmdir may still face physical whiteout residue
            // inside the upper dir (the visible-emptiness gate does not
            // count whiteouts) — sweep it before the physical rmdir. The
            // `EIO` arm is defensive: `is_pure_upper` already implies an
            // upper object.
            if kind == RemoveKind::Rmdir {
                let target_upper_dir = target_facts.upper().ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the pure-upper rmdir target has no upper real directory",
                    )
                })?;
                whiteout::cleanup_upper_whiteouts(&target_upper_dir.real_path()?)?;
            }
            // Step 3 — pure-upper direct removal, no whiteout: the name is
            // genuinely gone from the upper namespace, removed through the
            // base VFS `Path` layer. A physical-upper `ENOENT` means the
            // asserted upper object became stale behind the overlay and maps
            // to `ESTALE`; every other upper error propagates as-is. Both
            // publication steps are infallible, so no reconcile arm is
            // structurally reachable here.
            let result = if kind == RemoveKind::Rmdir {
                upper_parent_path.rmdir(name)
            } else {
                upper_parent_path.unlink(name)
            };
            result.map_err(translate_stale_upper_enoent)?;
            fs.bindings().invalidate(&self.key(), name);
            self.readdir_index_remove(name);
            // The removal may have restored purity — refresh the marker
            // best-effort (the mutation already committed; a refresh failure
            // never fails the removal).
            if let Err(err) = self.refresh_impure_marker() {
                warn!(
                    "overlay remove: the impure-marker refresh failed after the \
                     pure-upper removal (best-effort): {:?}",
                    err
                );
            }
            return Ok(());
        }

        // Step 4 — lower-backed target: preserve the lower result with a
        // published whiteout. `replace_target` tells `publish_whiteout`
        // (sibling `dir/whiteout.rs`) the physical shape of the name:
        // `None` (name absent in the upper → link a whiteout at it) vs
        // `Some(type_)` (present upper object → `Replace` non-dir /
        // `Exchange` + displaced-dir cleanup for a dir). The target type for
        // rmdir is always `Dir` (the non-dir type gate above refused
        // rmdir-on-file with `ENOTDIR` and unlink-on-dir with `EISDIR`).
        let target_type = target_inode.type_();
        let replace_target = target_facts.upper().map(|_| target_type);
        let clear_empty_temp = if kind == RemoveKind::Rmdir {
            match target_facts.upper() {
                Some(upper_obj) => {
                    let mut upper_names: Vec<String> = Vec::new();
                    upper_obj.real_inode().readdir_at(0, &mut upper_names)?;
                    upper_names.retain(|entry| !path::is_dot_or_dotdot(entry));
                    if upper_names.is_empty() {
                        None
                    } else {
                        let mode = upper_obj.real_inode().mode()?;
                        Some(fs.create_workdir_temp(
                            name,
                            &upper_parent_path,
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
        // The shared recipe scaffold: the commit marker is flipped at each
        // physical upper commit point and the reconcile / pre-publication
        // cleanup classification is owned by `run_recipe`.
        self.run_recipe(
            &fs,
            staged_temp,
            || self.invalidate_stale_cache(&[(self, name)]),
            |marker| {
                if let Some(temp) = clear_empty_temp.as_ref() {
                    // Clear-empty exchange (sibling helper): the upper
                    // directory of a lower-backed directory may hold entries
                    // that the merged view hides (whiteouts — the emptiness
                    // gate has already refused visible children). The helper
                    // replaces the upper dir with a workdir-prepared opaque
                    // temp dir (atomic exchange) and cleans the displaced old
                    // upper dir in the workdir; the whiteout is then
                    // published at the name by the recipe's common publish
                    // step below. The temp is never a visible source. The
                    // physical-upper commit is the exchange itself, so the
                    // commit marker flips immediately after the helper
                    // returns (a helper failure keeps the pre-commit
                    // classification for `run_recipe`'s best-effort cleanup).
                    self.clear_empty_exchange(
                        &fs,
                        &target_facts,
                        name,
                        &upper_parent_path,
                        temp.name(),
                        temp.inode(),
                    )?;
                    marker.commit();
                }
                // The whiteout publish (sibling `dir/whiteout.rs`): a present
                // non-dir upper object is replaced (`Replace`), a present dir
                // (the empty upper dir or the opaque temp) is exchanged and
                // its displaced form cleaned in the workdir (`Exchange`), and
                // an absent upper name gets a whiteout linked in (`link`).
                // Marker bytes are written by the sibling owner; no `WL`
                // payload is touched here.
                fs.publish_whiteout(&upper_parent_path, name, replace_target)
                    .map_err(|err| {
                        if replace_target.is_some() {
                            translate_stale_upper_enoent(err)
                        } else {
                            err
                        }
                    })?;
                marker.commit();
                // Semantic publication — inline composition: the whiteout
                // is re-observed from the upper (layer 0) so the published
                // `HiddenByWhiteout` binding pins its strong `HiddenEvidence`
                // barrier, then the parent index tombstones the now-hidden
                // name (the `readdir_index_remove` decision point). The
                // re-observation is fallible: on failure the whiteout is
                // already published — reconcile.
                let whiteout_path = Path::new(
                    upper_parent_path.mount_node().clone(),
                    upper_parent_path
                        .dentry()
                        .as_dir_dentry_or_err()?
                        .lookup_child(name)?,
                );
                let whiteout_inode = whiteout_path.inode().clone();
                let evidence = HiddenEvidence::new(0, whiteout_inode);
                fs.bindings().insert(
                    BindingKey::new(self.key(), String::from(name)),
                    Arc::new(Binding::Negative(NegativeBinding::HiddenByWhiteout(
                        evidence,
                    ))),
                );
                self.readdir_index_remove(name);
                Ok(())
            },
        )?;
        // The whiteout-publish removal may have restored purity — refresh
        // the marker best-effort (the mutation already committed; a refresh
        // failure never fails the removal).
        if let Err(err) = self.refresh_impure_marker() {
            warn!(
                "overlay remove: the impure-marker refresh failed after the \
                 whiteout publish (best-effort): {:?}",
                err
            );
        }
        Ok(())
    }

    /// Executes the clear-empty directory exchange of the lower-backed rmdir
    /// recipe.
    ///
    /// This helper factors the clear-empty exchange out of the recipe closure
    /// so the closure stays shallow, without changing the recipe semantics.
    /// When the upper directory of a lower-backed directory holds
    /// entries the merged view hides (necessarily whiteouts — the emptiness
    /// gate has already refused visible children), those entries must not
    /// leak and would defeat `publish_whiteout`'s displaced-dir workdir
    /// cleanup (`ENOTEMPTY`), so this helper replaces the upper dir with a
    /// workdir-prepared opaque temp dir (atomic `Exchange`), cleans the
    /// displaced old upper dir in the workdir, and lets the recipe's common
    /// publish step publish the whiteout over the opaque temp. The temp is
    /// never a visible source.
    ///
    /// The physical-upper commit point is the `Exchange` rename; the caller
    /// flips the [`CommitMarker`](crate::fs::fs_impls::overlayfs::copyup::promote::CommitMarker)
    /// immediately after this helper returns. The staged temp arrives as
    /// its `(name, inode)` parts, keeping this `dir`-module helper decoupled
    /// from the `copyup::workdir` handle type (the helper needs only the
    /// name and the staged inode). The displaced-dir cleanup is
    /// best-effort (warn-and-continue, never a visible entry); the one
    /// fallible re-observation (`as_dir_dentry_or_err`) is the defensive
    /// guard that the workdir staging workspace is a directory (it is, by
    /// `UpperWorkdirClaim::prepare_workdir` construction), and on that
    /// unreachable path the error propagates as a pre-commit failure; the
    /// commit marker has not yet flipped.
    fn clear_empty_exchange(
        &self,
        fs: &Arc<OverlayFs>,
        target_facts: &OverlayObjectFacts,
        name: &str,
        upper_parent_path: &Path,
        temp_name: &str,
        temp_inode: &Arc<dyn Inode>,
    ) -> Result<()> {
        let Some(upper_obj) = target_facts.upper() else {
            return Err(Error::with_message(
                Errno::EIO,
                "the clear-empty workdir temp has no upper directory",
            ));
        };
        // Clear-empty: the upper dir is replaced by a workdir-prepared opaque
        // temp dir (atomic exchange); the old upper dir is cleaned up in the
        // workdir; the whiteout is then published at the name by the recipe's
        // common publish step below. The temp is never a visible source.
        let old_upper_dir = upper_obj.real_inode().clone();
        // The opaque marker is part of the replacement directory's complete
        // preparation: it keeps the name a lower-search barrier at every
        // instant of the swap (crash window included), gated by the
        // private-xattr capability.
        let can_store_private_xattr = fs
            .policy()
            .upper_capabilities()
            .is_some_and(|caps| caps.can_store_private_xattr());
        if !can_store_private_xattr {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the opaque marker \
                         required for the clear-empty directory exchange",
            ));
        }
        let marker_name =
            XattrName::try_from_full_name(OPAQUE_XATTR_FULL_NAME).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "invalid overlay opaque marker xattr name")
            })?;
        let mut marker_reader = VmReader::from(OPAQUE_MARKER_VALUE).to_fallible();
        temp_inode.set_xattr(
            marker_name,
            &mut marker_reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )?;
        // The xattr buffer copy runs BEFORE the owner/group/mode are applied,
        // while the temp is still owned by the caller (the creating task), so
        // a non-owner rmdir of a directory carrying xattrs does not fail
        // `EACCES` on the temp `set_xattr`. `XattrName::try_from_full_name`
        // failure in the copy helper is `EINVAL` before its policy branch and
        // propagates. The remaining VFS list/read/write failures use the
        // BEST-EFFORT `XattrCopyPolicy::BestEffort` variant (the `ClearEmpty`
        // path): because the displaced upper dir is being deleted, they
        // degrade to warn-and-skip and the non-owner rmdir succeeds. See
        // `OverlayXattrPolicy::copy_eligible_xattrs` for the credential mechanism
        // and failure-policy discussion. The copy is filtered through the
        // `OverlayXattrPolicy` (private / escaped / reserved names never
        // copy; the temp's own markers are written explicitly by the recipe).
        fs.policy()
            .credential_policy()
            .with_creator_credentials_fn(|| {
                fs.xattr_policy().copy_eligible_xattrs(
                    &old_upper_dir,
                    temp_inode,
                    XattrCopyPolicy::BestEffort,
                )
            })?;
        // Metadata copy: owner/group/mode/times from the old upper dir onto
        // the temp.
        temp_inode.set_owner(old_upper_dir.owner()?)?;
        temp_inode.set_group(old_upper_dir.group()?)?;
        temp_inode.set_mode(old_upper_dir.mode()?)?;
        temp_inode.set_atime(old_upper_dir.atime());
        temp_inode.set_mtime(old_upper_dir.mtime());
        temp_inode.set_ctime(old_upper_dir.ctime());
        // Atomic exchange: the opaque temp becomes the upper object at `name`
        // and the old upper dir moves to the workdir temp name. From this
        // point the visible upper namespace has changed (reconcile applies on
        // any later failure). The workdir staging workspace resolves through
        // the single shared resolver (`OverlayInode::workdir_root_path`).
        let workdir_path = self.workdir_root_path()?;
        workdir_path
            .rename(temp_name, upper_parent_path, name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        // Clean the displaced old upper dir in the workdir: every remaining
        // entry is a whiteout (the emptiness gate refused visible children),
        // so sweep them through the shared path and rmdir the dir.
        // Best-effort: a cleanup failure is a known workdir-cleanup debt and
        // never becomes a visible namespace entry — the whiteout publish
        // below proceeds with the opaque temp at `name`. The displaced
        // directory now lives in the workdir under the temp name; its
        // dentry-anchored path is re-observed through the workdir dentry
        // layer so the sweep and the rmdir route through the base VFS view.
        match workdir_path
            .dentry()
            .as_dir_dentry_or_err()?
            .lookup_child(temp_name)
        {
            Ok(displaced_dentry) => {
                let displaced_path = Path::new(workdir_path.mount_node().clone(), displaced_dentry);
                if let Err(cleanup_err) = whiteout::cleanup_upper_whiteouts(&displaced_path) {
                    warn!(
                        "overlay clear-empty: the displaced-directory whiteout \
                         cleanup failed (residue, never a visible source): {:?}",
                        cleanup_err
                    );
                }
                if let Err(cleanup_err) = workdir_path.rmdir(temp_name) {
                    warn!(
                        "overlay clear-empty: workdir cleanup of the displaced \
                         directory {:?} failed (residue, never a visible source): {:?}",
                        temp_name, cleanup_err
                    );
                }
            }
            Err(reobserve_err) => {
                warn!(
                    "overlay clear-empty: re-observation of the displaced \
                     directory {:?} failed (residue, never a visible source): {:?}",
                    temp_name, reobserve_err
                );
            }
        }
        Ok(())
    }
}

/// Translates a physical-upper `ENOENT` into the stale-upper `ESTALE` error.
///
/// Used when the remove recipe's fresh projection asserted an upper object at
/// the target name (Linux `ovl_remove_upper` / `ovl_remove_and_whiteout`
/// return `ESTALE` when the upper dentry no longer matches); every other
/// errno passes through unchanged.
///
/// TODO(stale-upper): this post-operation errno translation is an indirect
/// approximation and is deliberately tricky. The faithful approach is a
/// VFS-level dentry verification: before the physical upper operation,
/// compare the overlay's cached upper dentry against a fresh upper lookup by
/// name and return `ESTALE` on mismatch without touching the upper (Linux
/// `ovl_matches_upper`). That requires a breaking VFS interface/behavior
/// change, which this change intentionally avoids; revisit once a non-breaking
/// VFS integration point exists.
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
