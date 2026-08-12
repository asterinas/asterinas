// SPDX-License-Identifier: MPL-2.0

//! The rename recipes.
//!
//! This module hosts the two rename-family recipe methods on [`OverlayInode`]:
//! the EXDEV gate ([`OverlayInode::cross_device_gate`], the
//! lower-backed/merged-directory cross-directory default) and the upper
//! rename recipe ([`OverlayInode::rename_upper`], same-directory and
//! cross-directory physical moves with the inline dual-parent publication).
//! The thin `Inode`-trait `rename` entry and the two-parent `DIR` lock helper
//! live
//! in the sibling `dir/mod.rs`; the entry holds both parent `DIR` domains,
//! runs the mutating admission per affected parent, derives the fresh source
//! projection, composes the EXDEV gate for cross-directory moves (before any
//! upper side effect), and delegates the per-branch promotion, the physical
//! upper rename, the source-whiteout compose, the inline dual-parent
//! publication, and the reconcile to this file.
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! Lock contract: the caller (the `dir/mod.rs` entry) holds both parent `DIR`
//! transaction locks in stable object-identity order and has pinned the
//! mount; this module acquires no Overlay lock of its own beyond the brief
//! `INODE` facts snapshots inside `facts_snapshot`/`select_real_inode`
//! (snapshot-and-release, never held across an underlying call) and the `CUL`
//! domains entered by the per-branch promotions (`ensure_upper_authority` for
//! the source object and the real admission stage —
//! `check_permission(AccessType::Mutating, ...)` — for each parent, under the
//! caller-held `DIR`s). The underlying upper/workdir operations
//! (`rename`/`lookup`/the whiteout publish) route through the base VFS
//! `Path` layer and run in the sleep-capable `DIR` domain under the
//! underlying filesystem's own locking; no `WL`/spin domain is entered and
//! no `WL` payload is touched here (the whiteout cache is the sibling
//! `dir/whiteout.rs` owner). `MOUNT` is never acquired; no Overlay lock
//! crosses the return boundary.
//!
//! Recipe notes:
//!
//! - **Source-whiteout compose:** Asterinas `RenameMode` has no
//!   `RENAME_WHITEOUT` flag (verified
//!   `kernel/src/fs/vfs/fs_apis/inode.rs:753-758`), so when the moved source
//!   had a lower fallback the source-name whiteout is a composed second upper
//!   step — the plain upper `rename`, then `publish_whiteout` at the old name
//!   — inside the same `DIR` domain(s). The intermediate (the lower name
//!   temporarily visible at the old position) is unobservable under `DIR` and
//!   is conservatively reconciled if the compose fails. A whiteout target
//!   inverts the compose: the rename switches whiteouts via
//!   `RenameMode::Exchange` (Linux `ovl_rename_start` "Switch whiteouts"),
//!   moving the target whiteout to the source name, so no composed second
//!   step is needed.
//! - **Target lower fallback:** after the move the moved source's upper
//!   object at the target name IS the target's hidden state — it covers the
//!   target's lower fallback exactly as Linux `ovl_rename_upper` publishes no
//!   target-name whiteout (`dir.c:1135-1339`). A literal target-name whiteout
//!   would hide the moved source and break the rename; the only whiteout this
//!   recipe publishes is the source-name compose. The `Replace`-mode overlay
//!   emptiness gate below is the merged-target check Linux runs instead
//!   (`ovl_check_empty_dir` in `ovl_rename_start`).
//! - **Redirect is not implemented:** no redirect option exists on the mount and
//!   no redirect xattr is written; the EXDEV default applies to every
//!   cross-directory lower-backed/merged directory source. Linux also sets an
//!   opaque marker on a pure-upper directory moved into a merged parent
//!   (`ovl_set_opaque_xerr`, `dir.c`); that marker write is not implemented
//!   and is a known Linux-fidelity gap for the redirect feature,
//!   alongside the `redirect_max`-style length obligation. The overlay-level
//!   emptiness gate keeps the moved-dir-over-lower-dir case from producing a
//!   wrong visible merge (`ENOTEMPTY`).
//! - **nlink bookkeeping:** Linux's `ovl_nlink_start`/`ovl_drop_nlink`
//!   accounting for replaced targets is not tracked in Asterinas (no overlay
//!   nlink model); the replaced target's upper inode simply loses its
//!   namespace name, matching the currently unsupported origin/index tracking.
//!
//! No `.unwrap()`/`.expect()` appears in any production path.

use super::whiteout;
use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::{
            AccessType,
            projection::{
                Binding, BindingKey, HiddenEvidence, NegativeBinding, OverlayInode, PositiveBinding,
            },
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Path,
        },
    },
    prelude::*,
};

impl OverlayInode {
    /// Returns `Err(EXDEV)` for a cross-directory move of a lower-backed or
    /// merged directory when the `redirect_dir` policy is not enabled.
    ///
    /// The gate runs from the fresh source projection **before any upper side
    /// effect** — no parent/source promotion, no workdir temp, no whiteout,
    /// no redirect xattr, no binding/index update. The caller (`dir/mod.rs`
    /// `Inode::rename`) composes the cross-directory condition ("different
    /// parents" — the signature carries no target, so the same-parent
    /// comparison is the entry's `DIR`-lock identity check) and invokes this
    /// gate only for a cross-directory move; this method checks the
    /// source-object side of the condition: a lower-backed or merged
    /// **directory** source. "Lower-backed or merged" is exactly
    /// `facts.lowers()` non-empty (a `Merged` directory has upper + lowers; a
    /// lower-backed `Single` directory has `upper == None`, `lowers[0]`; the
    /// facts invariant `upper.is_some() || !lowers.is_empty()` guarantees the
    /// empty-lowers case is genuinely upper-only).
    ///
    /// The `redirect_dir` policy is not implemented: no redirect
    /// mount option is published and no redirect xattr is ever written, so
    /// the EXDEV default applies; when redirect lands, the rejection below
    /// becomes the redirect-policy probe bounded by the `redirect_max`-style
    /// length rule.
    ///
    /// Lock contract: no Overlay lock is acquired or held; the caller holds
    /// both parent `DIR` domains. The `&self` receiver is the owner shape
    /// (the mutated directory is the recipe's natural owner); the gate's
    /// evidence is entirely the caller's fresh `source` binding.
    pub(super) fn cross_device_gate(&self, source: &Binding) -> Result<()> {
        // Only a directory source can be EXDEV-gated: same-directory and
        // non-directory moves always proceed. The `into_inode` route is the
        // only overlayfs-visible access to a positive binding's inode payload
        // (the field is projection-private); a negative binding has no inode
        // and never gates.
        let Some(source_inode) = source.clone().into_inode() else {
            return Ok(());
        };
        if !source_inode.type_().is_directory() {
            return Ok(());
        }
        // Lower-backed or merged: a lower object exists under the source
        // name (the empty-lowers case is a pure-upper directory, movable).
        if source_inode.facts_snapshot().lowers().is_empty() {
            return Ok(());
        }
        // The redirect policy is never enabled, so the EXDEV default fires
        // for every cross-directory lower-backed/merged directory source
        // (the entry composes the cross-directory condition). When redirect
        // lands, the policy probe replaces this rejection at this point.
        Err(Error::with_message(
            Errno::EXDEV,
            "the overlay cross-directory rename of a lower-backed or merged directory \
             requires the not-yet-implemented redirect_dir policy",
        ))
    }

    /// Runs the upper rename recipe — per-branch promotion, the physical
    /// upper rename (same-directory and cross-directory), the source-whiteout
    /// compose, and the inline dual-parent publication.
    ///
    /// The caller (the `dir/mod.rs` `Inode::rename` entry) holds both parent
    /// `DIR` domains and has run the mutating admission per affected parent
    /// and the EXDEV gate. "Source has a lower fallback" is derived inside
    /// this recipe from the freshly projected source facts — the entry passes
    /// no boolean. This recipe then:
    ///
    /// 1. **Re-derives the fresh source and target projections under `DIR`**
    ///    (the same binding-cache-first evidence the entry used — a negative
    ///    source is `ENOENT`, and a visible target under `NoReplace` is
    ///    `EEXIST` — and the `Replace`-mode merged-target emptiness gate,
    ///    Linux `ovl_check_empty_dir`).
    /// 2. **Promotes each branch in stable object-identity order:** the
    ///    source object (`ensure_upper_authority`), then the source parent,
    ///    then the target parent (via `check_permission(AccessType::Mutating,
    ///    ...)`); each branch's scope is decided under its own `CUL`, and the
    ///    entry's earlier admission makes these idempotent no-ops in the
    ///    ordinary path.
    /// 3. **Performs the physical upper rename** — same-directory
    ///    `upper_parent_path.rename(old, upper_parent_path, new, ...)`,
    ///    cross-directory
    ///    `upper_parent_path.rename(old, target_upper_parent_path, new,
    ///    ...)` — with the `RenameMode` (Replace/NoReplace/Exchange per
    ///    `mode`) and the whiteout-target adjustments of Linux
    ///    `ovl_rename_start` (consume/replace a whiteout marker; switch
    ///    whiteouts via `Exchange` when the source has a lower fallback), and
    ///    then the source-whiteout compose when the moved source had a lower
    ///    fallback (Asterinas has no `RENAME_WHITEOUT`; the compose is a
    ///    second upper step inside the same `DIR` domain(s)).
    /// 4. **Publishes inline:** the source
    ///    binding (`BindingCache::invalidate`, or a
    ///    `Negative(HiddenByWhiteout)` insert when a source whiteout was
    ///    published, pinning the whiteout's real inode via `HiddenEvidence`
    ///    with layer index 0), the target binding (`BindingCache::insert`
    ///    positive, sharing the moved source `OverlayInode` with the kind
    ///    derived from the source inode's own facts — `lookup_binding`
    ///    derives the per-name kind from the projected facts, so the
    ///    published binding mirrors the object's classification), and
    ///    `invalidate_readdir_index` on both affected parents (same parent
    ///    once; rename is a reordering operation).
    ///
    /// Any failure after the physical upper rename committed — the
    /// source-whiteout compose or the hidden-binding evidence re-lookup —
    /// triggers the conservative reconcile of the whole affected set as a
    /// unit before the error is returned.
    ///
    /// Lock contract: runs under the caller's two parent `DIR` domains; the
    /// promotions enter the per-branch `CUL` → `INODE` domains in order; the
    /// underlying upper operations may block and run in the sleep-capable
    /// domain, never under `WL` or any spin lock. No Overlay lock is acquired
    /// or held by this method and none crosses the return boundary.
    pub(super) fn rename_upper(
        &self,
        old_name: &str,
        target: &Arc<OverlayInode>,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        let fs = self.fs_arc()?;

        // Fresh source and target projections under the caller-held `DIR`
        // domain(s) (never from a stale VFS dentry). The source must be
        // visible — the `DIR`-domain projection is authoritative over the VFS
        // dentry that may have triggered the call.
        let source_binding = fs.lookup_binding(&self.facts_snapshot(), old_name)?.binding;
        let source_inode = source_binding.clone().into_inode().ok_or_else(|| {
            Error::with_message(
                Errno::ENOENT,
                "the rename source is not visible under the parent DIR",
            )
        })?;
        // "Source has a lower fallback" decides whether the source name gets
        // a whiteout after the move. The signal is derived HERE from the
        // freshly projected source facts — the entry passes no bare bool —
        // and `lowers` is retained across copy-up, so the value is stable
        // through the per-branch promotion below.
        let source_has_lower = !source_inode.facts_snapshot().lowers().is_empty();
        let target_binding = fs
            .lookup_binding(&target.facts_snapshot(), new_name)?
            .binding;
        let target_is_whiteout = matches!(
            &target_binding,
            Binding::Negative(NegativeBinding::HiddenByWhiteout(_))
        );
        let target_is_positive = matches!(&target_binding, Binding::Positive(_));

        // A visible target under `NoReplace` is `EEXIST` (the upper rename's
        // NOREPLACE only observes the upper namespace — a lower-visible name
        // must still fail, the Linux `ovl_copy_up(new)` equivalence); the
        // fresh projection is authoritative and no upper side effect runs.
        if mode == RenameMode::NoReplace && target_is_positive {
            return Err(Error::with_message(
                Errno::EEXIST,
                "the rename target already exists and is visible",
            ));
        }

        // `Replace` over a visible lower-backed directory target requires the
        // merged target directory to be overlay-visible-empty before the move
        // (Linux `ovl_check_empty_dir` in `ovl_rename_start`; the upper
        // rename only sees the upper dir). The `visible_child_count` integration point
        // counts visible children (whiteout-hidden children do not count); a
        // pure-upper target defers to the upper rename's own emptiness
        // enforcement. The gate records the fresh target facts so the
        // target's physical whiteout-residue sweep can run on its physical
        // upper copy after the per-branch promotions.
        let gate_target_facts = if mode == RenameMode::Replace
            && target_is_positive
            && let Some(target_object) = target_binding.clone().into_inode()
            && target_object.type_().is_directory()
        {
            let target_facts = target_object.facts_snapshot();
            if !target_facts.lowers().is_empty()
                && target_object.visible_child_count(&target_facts)? != 0
            {
                return Err(Error::with_message(
                    Errno::ENOTEMPTY,
                    "the overlay rename target directory is not empty",
                ));
            }
            Some(target_facts)
        } else {
            None
        };

        // Per-branch promotion in stable object-identity order: the source
        // object first, then the source parent, then the target parent. Each
        // branch's scope is decided under its own `CUL`;
        // `ensure_upper_authority` and the real admission promotion are
        // idempotent fast paths when the branch is already upper-backed (the
        // entry's admission already promoted both parents, so these are
        // no-ops in the ordinary path).
        source_inode.ensure_upper_authority()?;
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        target.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;

        // The promoted upper real parents — the physical-operation targets
        // (post-promotion dentry-anchored `Path`s via
        // `dir/mod.rs::upper_parent_path`).
        let upper_parent_path = self.upper_parent_path()?;
        let target_upper_parent_path = target.upper_parent_path()?;

        // When the Replace gate passed for a directory target with a
        // physical upper copy, sweep the target's physical whiteout residue
        // before the physical rename (Linux `ovl_clear_empty` for the same
        // case, dir.c:1215-1227). Strict and pre-commit: a failure aborts
        // before `run_recipe`, whose pre-commit cleanup is a no-op because
        // no workdir temp is staged.
        if let Some(target_upper_dir) = gate_target_facts
            .as_ref()
            .and_then(|target_facts| target_facts.upper())
        {
            whiteout::cleanup_upper_whiteouts(&target_upper_dir.real_path()?)?;
        }

        // A cross-directory move of an origin-bearing source makes the
        // target parent impure — persist the marker before the physical
        // rename (Linux `ovl_rename_upper` cross-dir origin arm; strict,
        // pre-commit).
        let same_parent = self.key() == target.key();
        if !same_parent && source_has_lower {
            fs.xattr_policy()
                .set_impure_marker(target_upper_parent_path.inode())?;
        }

        // The shared recipe scaffold: the commit marker is flipped at the
        // physical upper rename and the reconcile classification is owned by
        // `run_recipe`; the plain upper rename stages no workdir temp, so
        // `temp_name` is `None` (a pre-commit failure has nothing to clean).
        self.run_recipe(
            &fs,
            None,
            || self.invalidate_stale_cache(&[(target.as_ref(), new_name), (self, old_name)]),
            |marker| {
                // Whiteout-target adjustments (Linux `ovl_rename_start`): a
                // whiteout is a negative name — never a visible NOREPLACE
                // failure and never an ordinary rename target — so it is
                // always replaced or switched: a source with a lower fallback
                // switches whiteouts via `Exchange` (the whiteout lands at the
                // source name, so the composed second step is not needed); any
                // other source consumes the marker with a plain replace. A
                // caller-requested `Exchange` is preserved.
                let effective_mode = match mode {
                    RenameMode::Exchange => RenameMode::Exchange,
                    _ if target_is_whiteout && source_has_lower => RenameMode::Exchange,
                    _ if target_is_whiteout => RenameMode::Replace,
                    _ => mode,
                };
                // The physical upper rename: same-directory against the
                // single upper parent, cross-directory against the promoted
                // target upper parent — through the base VFS `Path` layer.
                if same_parent {
                    upper_parent_path.rename(
                        old_name,
                        &upper_parent_path,
                        new_name,
                        effective_mode,
                    )?;
                } else {
                    upper_parent_path.rename(
                        old_name,
                        &target_upper_parent_path,
                        new_name,
                        effective_mode,
                    )?;
                }
                marker.commit();
                // Source-whiteout compose: when the moved source had a lower
                // fallback and the move vacated the source name without
                // leaving a cover (neither a switched whiteout nor a
                // caller-requested exchange), the source-name whiteout is the
                // composed second upper step inside the same `DIR` domain(s).
                // The intermediate (the lower name temporarily visible at the
                // old position) is unobservable under `DIR` and is
                // conservatively reconciled if the compose fails.
                let mut source_whiteout_published = false;
                if source_has_lower && !target_is_whiteout && mode != RenameMode::Exchange {
                    fs.publish_whiteout(&upper_parent_path, old_name, None)?;
                    source_whiteout_published = true;
                }
                // Dual-parent publication (inline; the `publish_rename`
                // helper is dissolved). Source binding: a published whiteout
                // is inserted as the hidden barrier binding (pinning the
                // whiteout's real inode via `HiddenEvidence`, layer index 0 =
                // upper); a plain move leaves the old name vacated, so the
                // stale positive binding is invalidated and the next lookup
                // re-derives from upper truth.
                if source_whiteout_published {
                    let whiteout_path = Path::new(
                        upper_parent_path.mount_node().clone(),
                        upper_parent_path
                            .dentry()
                            .as_dir_dentry_or_err()?
                            .lookup_child(old_name)?,
                    );
                    let whiteout_real = whiteout_path.inode().clone();
                    fs.bindings().insert(
                        BindingKey::new(self.key(), String::from(old_name)),
                        Arc::new(Binding::Negative(NegativeBinding::HiddenByWhiteout(
                            HiddenEvidence::new(0, whiteout_real),
                        ))),
                    );
                } else {
                    fs.bindings().invalidate(&self.key(), old_name);
                }
                // Target binding: the moved source is now the visible object
                // at the target name; its classification remains in the source
                // inode's own facts, so the published binding has no stale
                // per-name classification snapshot.
                fs.bindings().insert(
                    BindingKey::new(target.key(), String::from(new_name)),
                    Arc::new(Binding::Positive(PositiveBinding::new(
                        source_inode.clone(),
                    ))),
                );
                // Rename reorders the visible sequence; the conservative rule
                // invalidates on every affected parent (same parent once).
                self.invalidate_readdir_index();
                if !same_parent {
                    target.invalidate_readdir_index();
                }
                Ok(())
            },
        )?;
        // A cross-directory rename may have restored purity in the source or
        // target parent (the overwrite-of-origin-target case can clear the
        // target's last origin-bearing entry) — refresh both markers
        // best-effort (the mutation already committed; a refresh failure
        // never fails the rename).
        if !same_parent {
            if let Err(err) = self.refresh_impure_marker() {
                warn!(
                    "overlay rename: the source-parent impure-marker refresh failed \
                     (best-effort): {:?}",
                    err
                );
            }
            if let Err(err) = target.refresh_impure_marker() {
                warn!(
                    "overlay rename: the target-parent impure-marker refresh failed \
                     (best-effort): {:?}",
                    err
                );
            }
        }
        Ok(())
    }
}
