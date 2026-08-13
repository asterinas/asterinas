// SPDX-License-Identifier: MPL-2.0

//! The shared whiteout cache and whiteout-publish mechanics.
//!
//! This module owns the `dir/whiteout.rs` surface on [`OverlayFs`] and
//! [`WhiteoutCache`]: the mount-scoped `WL` (whiteout-lock) payload
//! ([`WhiteoutCache`], bounded to one cached workdir whiteout), the
//! capability-derived [`WhiteoutRepresentation`] (`can_mknod_char` →
//! `CharDevice`, else `can_store_private_xattr` → `Xattr`; **no runtime
//! probe**), the cached-item shape ([`WhiteoutHandle`]), the private temp
//! creation ([`OverlayFs::create_whiteout_temp`]), the publish entry
//! ([`OverlayFs::publish_whiteout`]), and the short
//! `take`/`store`/`disable_sharing` slot protocol. The sibling `dir/remove.rs`
//! and `dir/rename.rs` compose `publish_whiteout` for the lower-backed
//! removal and rename-source-cleanup whiteouts; `create.rs`/`link.rs` consume
//! the opaque/whiteout replacement semantics without touching this module's
//! payload.
//!
//! Lock domains: `DIR` = per-parent directory transaction lock; `CUL` =
//! per-object copy-up lock; `INODE` = per-object facts lock; `WL` =
//! whiteout-cache lock; `MOUNT` = mount-lifecycle lock; `UPPER` =
//! underlying upper-filesystem lock; `IU` = mount-time upper/workdir
//! in-use claim.
//!
//! # The `WL` lock domain
//!
//! The single `WL` payload is `OverlayFs::whiteout_cache: Mutex<WhiteoutCache>`
//! (the field in `mount/superblock.rs`). `WL` critical sections are
//! the **short slot operations only** — `take`/`store`/`disable_sharing`
//! (pop / push / flag) — and never contain BIO, sleeping allocation,
//! underlying VFS calls, callbacks, or waits. All fallible and sleep-capable
//! work — temp creation (`mknod`/`create` + the whiteout-marker xattr write),
//! the underlying `link`, and the workdir `rename` — runs **outside** `WL` in
//! the caller's sleep-capable `DIR` domain. This field deliberately uses a
//! `Mutex` rather than an `RwMutex`; the choice may be revisited.
//!
//! # Representation derivation
//!
//! The whiteout physical form is derived, never probed:
//! `OverlayFs::whiteout_representation` returns `CharDevice` when the
//! capability `can_mknod_char` is set, else `Xattr` when
//! `can_store_private_xattr` is set, else the defensive `EOPNOTSUPP`
//! (unreachable for a writable overlay with lowers per the whiteout-
//! capability mount gate). The representation is deliberately **not** stored
//! on the cache (no duplicate state; the enum classifies the two closed
//! physical forms).
//!
//! # Invariants
//!
//! - At most one cached whiteout (`cached: Option<WhiteoutHandle>`); a cached
//!   whiteout is a workdir object that is never a directory entry of any
//!   upper parent nor a `ReaddirIndex` source.
//! - `can_share_by_link == false` implies future publishes use rename-over
//!   move semantics (set once on `EMLINK`/`EOPNOTSUPP`; never re-enabled).
//! - A published whiteout is a visibility barrier, never an inode: the
//!   publish entry only produces the upper object; the
//!   `HiddenByWhiteout(HiddenEvidence)` binding publication is the sibling
//!   recipe's inline publication step.
//! - No `.unwrap()`/`.expect()` in any production path (hard invariant
//!   failures use the `unreachable!`/error-return precedents).

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            copyup::WorkdirTempRequest, metadata_security::xattr::WHITEOUT_XATTR_FULL_NAME,
            mount::OverlayFs, projection::is_whiteout_inode,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::{self, Path},
            xattr::{XattrName, XattrSetFlags},
        },
    },
    prelude::*,
};

/// The marker value of the xattr-based whiteout (the single byte `"y"`).
///
/// The whiteout reader is presence-based; the first byte `b'y'` is the Linux
/// `OVL_XATTR_XWHITEOUT` value (confirmed against `projection/entry.rs`).
const WHITEOUT_MARKER_VALUE: &[u8] = b"y";

/// The classic-whiteout char device `0:0` (Linux `OVL_XATTR` whiteout).
///
/// The device number `0` is the `makedev(0, 0)` encoding of the kernel's
/// whiteout device identity. The xattr reader is presence-based and never
/// inspects the number, but the char-device whiteout form is exactly this
/// contract.
const WHITEOUT_CHAR_DEV: u64 = 0;

/// The target-name component of workdir whiteout temp names.
///
/// `create_whiteout_temp` takes no name argument, yet the naming helper
/// `generate_workdir_temp_name(target_name, upper_parent)` requires a
/// target-name component; the cached whiteout is a generic workdir resource —
/// not a `(parent, name)` owner — so a fixed content-named component is used.
/// Uniqueness comes from the composite (`#{name}#{parent_ino}#{serial}`):
/// the workdir staging-workspace real ino plus the per-mount saturating
/// `workdir_temp_serial` make the name unique per mount (the claim protocol
/// guarantees no cross-mount collision).
const WHITEOUT_TEMP_NAME_COMPONENT: &str = "whiteout";

/// The mount-scoped reusable whiteout cache — the `WL` payload.
///
/// Bounded to one reusable workdir whiteout (private staging) plus the
/// share-by-link flag. Invariants: at most one cached whiteout; a cached
/// whiteout is a workdir object that is never a directory entry of any upper
/// parent nor a `ReaddirIndex` source; `can_share_by_link == false` implies
/// future publishes use rename-over. The whiteout *representation* is NOT
/// stored here (no duplicate state): it is derived on demand from the
/// immutable published capabilities via [`OverlayFs::whiteout_representation`].
///
/// Stored at `OverlayFs::whiteout_cache: Mutex<WhiteoutCache>` — the `WL`
/// domain, a sleep-capable `ostd::sync::Mutex` whose critical sections never
/// contain BIO/sleep/underlying calls/callbacks/waits (the cache-slot
/// protocol). The cache field deliberately uses a `Mutex` rather than an
/// `RwMutex`; the choice may be revisited. The cache-slot fields stay
/// private; the only external surface is the constructor and the slot
/// methods used by this file's `publish_whiteout`.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct WhiteoutCache {
    /// The single reusable workdir whiteout (private staging); `None` when
    /// the slot is empty. Bounded to 1.
    cached: Option<WhiteoutHandle>,
    /// `true` initially; set `false` on `EMLINK`/`EOPNOTSUPP` (move
    /// semantics, Linux `no_shared_whiteout`, `dir.c:77-119`).
    can_share_by_link: bool,
}

impl WhiteoutCache {
    /// Constructs the empty cache slot (`cached: None`,
    /// `can_share_by_link: true`).
    ///
    /// Called by `OverlayFs::new` (`mount/build.rs`); the constructor is the
    /// single construction path outside this module.
    pub(in crate::fs::fs_impls::overlayfs) fn new() -> Self {
        Self {
            cached: None,
            can_share_by_link: true,
        }
    }

    /// Pops the cached whiteout handle, if any (the `WL` slot-pop).
    ///
    /// Short critical section only: no BIO/sleep/underlying call under `WL`.
    /// The protocol takes before storing, so the slot is empty after a
    /// successful take.
    fn take(&mut self) -> Option<WhiteoutHandle> {
        self.cached.take()
    }

    /// Pushes a whiteout handle back into the cache (the `WL` slot-push).
    ///
    /// Bounded to one slot: the protocol pops before publishing and re-stores
    /// only the workdir original kept alive by the link path, so an occupied
    /// slot here is a protocol violation; the stale handle is dropped (its
    /// workdir object becomes known workdir-cleanup residue, never a visible
    /// source) rather than exceeding the bound. Short critical section only.
    fn store(&mut self, handle: WhiteoutHandle) {
        if self.cached.replace(handle).is_some() {
            warn!(
                "overlay whiteout cache slot occupied at store; the stale cached whiteout is \
                 dropped (workdir-cleanup residue, never a visible source)"
            );
        }
    }

    /// Disables whiteout sharing by link (the `WL` fallback flag).
    ///
    /// Set on `EMLINK`/`EOPNOTSUPP` from the link path; once `false`, every
    /// future publish uses rename-over move semantics. Never re-enabled.
    /// Short critical section only.
    fn disable_sharing(&mut self) {
        self.can_share_by_link = false;
    }
}

/// One cached or mutation-local workdir whiteout (the `WL` cached-item shape).
///
/// `inode` is the whiteout object — a char `0:0` device or a zero-size
/// regular file carrying the `trusted.overlay.whiteout` marker (inode
/// identity equal to `path.inode()`), `workdir_name` is its name in the
/// workdir (needed for rename-over publishes), and `path` is its
/// dentry-anchored workdir temp `Path` (the `Path::link`/`Path::rename`
/// publish arms route through the base VFS dentry layer so the base view
/// observes the published whiteout). Invariants: `workdir_name` is non-empty
/// and unique (generated via `generate_workdir_temp_name`); the handle never
/// outlives its use in one mutation unless re-cached.
///
/// Owned by `WhiteoutCache::cached` or a mutation-local; the strong inode pin
/// keeps the workdir object alive. Only the sibling recipes in `dir` name
/// this shape.
#[derive(Debug)]
pub(super) struct WhiteoutHandle {
    /// The whiteout object (char `0:0` device or zero-size file + whiteout
    /// xattr); a strong pin keeps the workdir object alive.
    #[expect(
        dead_code,
        reason = "retained strong pin: the strong inode pin keeps the workdir object alive \
                  while the dentry-anchored `path` routes the publish arms"
    )]
    inode: Arc<dyn Inode>,
    /// Its name in the workdir; needed for rename-over publishes.
    workdir_name: String,
    /// The dentry-anchored workdir temp path of the whiteout; the
    /// `Path::link`/`Path::rename` publish arms route through it.
    path: Path,
}

/// The closed set of physical whiteout forms.
///
/// `CharDevice`: the classic whiteout — a char device `0:0` created by
/// workdir `mknod`. `Xattr`: a zero-size regular file carrying the
/// `trusted.overlay.whiteout` marker, requiring `can_store_private_xattr`.
/// The choice is DERIVED from the published capabilities
/// (`OverlayFs::whiteout_representation()`: `can_mknod_char` → `CharDevice`,
/// else `can_store_private_xattr` → `Xattr`) — there is NO runtime probe and
/// NO per-mount cached copy. The enum (not a bare bool) classifies the closed
/// pair because the two forms carry different recipe behavior (mknod vs
/// create+xattr).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WhiteoutRepresentation {
    /// Classic whiteout: char device `0:0` (workdir mknod).
    CharDevice,
    /// Xattr whiteout: zero-size regular file + `trusted.overlay.whiteout`
    /// (needs `can_store_private_xattr`).
    Xattr,
}

impl OverlayFs {
    /// Returns the mount-scoped whiteout cache — the `WL` domain.
    ///
    /// The cache is the only `WL` payload; the slot protocol
    /// (`take`/`store`/`disable_sharing`) is the only `WL` critical section
    /// and never covers BIO/sleep/underlying calls. The sibling `dir` recipes
    /// use it to reach the domain without touching the field; the cache slot
    /// itself is only ever manipulated by this file's `publish_whiteout`.
    pub(super) fn whiteout_cache(&self) -> &Mutex<WhiteoutCache> {
        &self.whiteout_cache
    }

    /// Derives the whiteout representation from the published capabilities
    /// (no runtime probe).
    ///
    /// `can_mknod_char` → [`WhiteoutRepresentation::CharDevice`]; else
    /// `can_store_private_xattr` → [`WhiteoutRepresentation::Xattr`]; else
    /// the defensive `EOPNOTSUPP` (unreachable for a writable overlay with
    /// lowers per the whiteout-capability mount gate). A missing capability
    /// snapshot means the mount has no writable claim (the snapshot is probed
    /// at mount time for writable mounts only), so the defensive arm is
    /// `EROFS` — the same writable-state error the admission gate and the
    /// `workdir_root_path` resolver use; both arms are unreachable for a
    /// published writable overlay.
    fn whiteout_representation(&self) -> Result<WhiteoutRepresentation> {
        let capabilities = self.policy().upper_capabilities().ok_or_else(|| {
            Error::with_message(
                Errno::EROFS,
                "the overlay mount has no writable upper capability snapshot",
            )
        })?;
        if capabilities.can_mknod_char() {
            Ok(WhiteoutRepresentation::CharDevice)
        } else if capabilities.can_store_private_xattr() {
            Ok(WhiteoutRepresentation::Xattr)
        } else {
            Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form (neither char-device mknod \
                 nor private xattr)",
            ))
        }
    }

    /// Creates one private workdir whiteout temp outside `WL` (BIO-capable).
    ///
    /// `representation := whiteout_representation()?`; the shared
    /// [`OverlayFs::create_workdir_temp`] entry generates a unique name for
    /// every attempt using `WHITEOUT_TEMP_NAME_COMPONENT` (the cached
    /// whiteout is a generic workdir resource, not a `(parent, name)` owner).
    /// `CharDevice` uses the typed `Mknod` request; `Xattr` uses `Create` then
    /// `set_xattr("trusted.overlay.whiteout", "y", CREATE_OR_REPLACE)` — the
    /// marker write is the owning operation (the name is verified through the
    /// `OverlayXattrPolicy::is_private` classification as a `debug_assert!`
    /// hard invariant, and the `Xattr` path is gated by
    /// `can_store_private_xattr` through the representation derivation). On an
    /// xattr-write failure the created temp is removed best-effort
    /// (`cleanup_workdir_temp`) so no workdir residue outlives the failed
    /// creation (never a visible source).
    fn create_whiteout_temp(&self) -> Result<WhiteoutHandle> {
        let representation = self.whiteout_representation()?;
        // The workdir staging workspace resolves through the single shared
        // resolver (`OverlayFs::workdir_root_path`).
        let workdir_path = self.workdir_root_path()?;
        match representation {
            WhiteoutRepresentation::CharDevice => {
                let node = MknodType::CharDevice(WHITEOUT_CHAR_DEV);
                let (workdir_name, path) = self
                    .create_workdir_temp(
                        WHITEOUT_TEMP_NAME_COMPONENT,
                        &workdir_path,
                        WorkdirTempRequest::Mknod {
                            mode: InodeMode::empty(),
                            node: &node,
                        },
                    )?
                    .into_parts();
                Ok(WhiteoutHandle {
                    inode: path.inode().clone(),
                    workdir_name,
                    path,
                })
            }
            WhiteoutRepresentation::Xattr => {
                // The zero-size regular file carries the whiteout marker
                // (Linux `OVL_XATTR_XWHITEOUT`); both the marker spelling and
                // the classification are the owning operation. The
                // representation derivation already gated this branch on
                // `can_store_private_xattr`.
                debug_assert!(
                    self.xattr_policy().is_private(WHITEOUT_XATTR_FULL_NAME),
                    "the whiteout marker name must classify as an overlay-private record"
                );
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    &workdir_path,
                    WorkdirTempRequest::Create {
                        kind: InodeType::File,
                        mode: InodeMode::empty(),
                    },
                )?;
                let marker_name = XattrName::try_from_full_name(WHITEOUT_XATTR_FULL_NAME)
                    .ok_or_else(|| {
                        Error::with_message(
                            Errno::EINVAL,
                            "invalid overlay whiteout marker xattr name",
                        )
                    })?;
                let mut marker_reader = VmReader::from(WHITEOUT_MARKER_VALUE).to_fallible();
                if let Err(err) = temp.inode().set_xattr(
                    marker_name,
                    &mut marker_reader,
                    XattrSetFlags::CREATE_OR_REPLACE,
                ) {
                    // Best-effort temp cleanup on the pre-publication failure
                    // (the cleanup debt never becomes a visible entry).
                    let _ = self.cleanup_workdir_temp(temp.name(), temp.kind());
                    return Err(err);
                }
                let (workdir_name, path) = temp.into_parts();
                Ok(WhiteoutHandle {
                    inode: path.inode().clone(),
                    workdir_name,
                    path,
                })
            }
        }
    }

    /// Publishes a whiteout at `(upper_parent_path, name)`.
    ///
    /// Obtains a whiteout (`WL` pop of the cached handle, or a fresh
    /// [`OverlayFs::create_whiteout_temp`] outside `WL`), then publishes at
    /// `(upper_parent_path, name)` per `replace_target`:
    ///
    /// - `None` (target absent) →
    ///   `upper_parent_path.link(&handle.path, name)` keeps the workdir
    ///   original, which is re-stored under `WL` (bounded to 1).
    ///   `EMLINK`/`EOPNOTSUPP` on the link path → `disable_sharing` under
    ///   `WL` and a retry with move semantics (rename-over; Linux
    ///   `no_shared_whiteout`, `dir.c:77-119`). When sharing is already
    ///   disabled the publish starts directly with the rename-over.
    /// - `Some(non-dir)` (target present) →
    ///   `workdir_path.rename(temp_name, upper_parent_path, name, Replace)`;
    ///   the whiteout is consumed, no re-cache.
    /// - `Some(Dir)` (target present) →
    ///   `workdir_path.rename(temp_name, upper_parent_path, name, Exchange)`
    ///   — the displaced directory lands in the workdir at the temp name —
    ///   then best-effort workdir `rmdir` cleanup of the displaced dir
    ///   (clear-empty/rmdir paths); a cleanup failure is a known
    ///   workdir-cleanup debt and never a visible namespace entry (the
    ///   whiteout is already published, so the semantic publish succeeded).
    ///
    /// The whiteout marker bytes are written by the owning operation inside
    /// `create_whiteout_temp` (the `Xattr` form), which `publish_whiteout`
    /// invokes for a fresh temp; a cached whiteout carries the marker from
    /// its creation, so every published object carries it before the link/
    /// rename. All physical operations route through the base VFS `Path`
    /// layer, so the base view observes the published whiteout. Runs in the
    /// sleep-capable `DIR` domain of the caller; `WL` is held only for the
    /// short slot operations.
    pub(super) fn publish_whiteout(
        &self,
        upper_parent_path: &Path,
        name: &str,
        replace_target: Option<InodeType>,
    ) -> Result<()> {
        // Step 1 — the `WL` cache-slot pop: read `can_share_by_link` and take
        // the cached handle under `WL`, then release `WL` before any
        // fallible/BIO-capable work. The block scope drops the guard before
        // the temp creation below.
        let (cached, can_share_by_link) = {
            let mut cache = self.whiteout_cache().lock();
            let cached = cache.take();
            let can_share_by_link = cache.can_share_by_link;
            (cached, can_share_by_link)
        };

        // Step 2 — obtain the whiteout handle OUTSIDE `WL` (BIO-capable:
        // workdir mknod/create + the marker xattr write).
        let handle = match cached {
            Some(handle) => handle,
            None => self.create_whiteout_temp()?,
        };

        // Step 3 — publish at `(upper_parent_path, name)`. The workdir
        // staging workspace is the physical rename source; a missing
        // writable claim is the EROFS gate (the admission already passed for
        // a live mutation, so this is the defensive arm) — resolved through
        // the single shared resolver (`OverlayFs::workdir_root_path`).
        let workdir_path = self.workdir_root_path()?;
        // Publishing a whiteout inside the parent makes it impure, so the
        // marker is refreshed before the physical publish (read-first
        // idempotent; the marker write is a raw-inode xattr op on the
        // resolved upper parent inode). The marker is a cache hint whose
        // consumer refreshes it best-effort, so a marker failure must not
        // abort the physical publish: warn and continue (an upper that can
        // host whiteouts but cannot store private xattrs keeps lower-backed
        // removals usable instead of failing them outright).
        if let Err(err) = self
            .xattr_policy()
            .set_impure_marker(upper_parent_path.inode())
        {
            warn!(
                "overlay whiteout publish: failed to set the impure marker on {:?} \
                 (best-effort cache hint; continuing with the physical publish): {:?}",
                upper_parent_path.inode(),
                err
            );
        }
        match replace_target {
            // Target absent: the link path keeps the workdir original for
            // reuse (share); a link that fails the share contract degrades to
            // move semantics.
            None => {
                if can_share_by_link {
                    match upper_parent_path.link(&handle.path, name) {
                        Ok(()) => {
                            // Re-store the workdir original under `WL`
                            // (bounded to 1); the link succeeded, so the
                            // whiteout object is shared, not consumed.
                            self.whiteout_cache().lock().store(handle);
                            return Ok(());
                        }
                        Err(err) if matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP) => {
                            // Linux `no_shared_whiteout`: disable sharing and
                            // retry with rename-over (move semantics).
                            self.whiteout_cache().lock().disable_sharing();
                        }
                        Err(err) => return Err(err),
                    }
                }
                workdir_path.rename(
                    &handle.workdir_name,
                    upper_parent_path,
                    name,
                    RenameMode::Replace,
                )?;
                Ok(())
            }
            // Target present (non-dir): rename the whiteout over it
            // (`Replace`); the whiteout is consumed, never re-cached.
            Some(target_type) if !target_type.is_directory() => {
                workdir_path.rename(
                    &handle.workdir_name,
                    upper_parent_path,
                    name,
                    RenameMode::Replace,
                )?;
                Ok(())
            }
            // Target present (dir): `Exchange` swaps the whiteout into the
            // name and the displaced directory into the workdir; the displaced
            // dir is then cleaned up best-effort (clear-empty/rmdir paths).
            // The whiteout is consumed, never re-cached.
            Some(_) => {
                workdir_path.rename(
                    &handle.workdir_name,
                    upper_parent_path,
                    name,
                    RenameMode::Exchange,
                )?;
                if let Err(cleanup_err) = workdir_path.rmdir(&handle.workdir_name) {
                    warn!(
                        "overlay whiteout publish: workdir cleanup of the displaced directory \
                         {:?} failed (residue, never a visible source): {:?}",
                        handle.workdir_name, cleanup_err
                    );
                }
                Ok(())
            }
        }
    }
}

/// Sweeps the physical whiteout residue out of an upper directory.
///
/// Enumerates the real upper directory through a raw inode `readdir` (the
/// allowed read class), filters `.`/`..`, and runs a two-phase sweep. The
/// first phase validates every physical child: each name is re-observed with
/// `lookup_child` and classified with the shared [`is_whiteout_inode`]
/// predicate (char device `0:0` or the `trusted.overlay.whiteout` marker
/// value `b'y'`), and any non-whiteout child refuses the whole sweep with
/// `ENOTEMPTY` (Linux `ovl_check_empty_dir` parity — unknown state is never
/// deleted) **before any entry is removed**. The second phase re-observes and
/// re-classifies each child immediately before its `unlink`, so an entry
/// replaced between the two phases is detected instead of deleted. The two
/// phases are not atomic: a concurrent modification of the upper directory
/// (an overlay mutation or a direct upper-layer writer that lives outside
/// every overlay lock domain) can still slip an entry in between the
/// phase-2 check and the unlink, so the sweep relies on the upper directory
/// not being modified concurrently and the re-check only shortens that
/// residual window. Underlying lookup/readdir/unlink errors propagate
/// unchanged. The sweep never recurses into directories: with the caller's
/// visible-emptiness gate holding, every deleted physical child is a
/// whiteout, so the bound is one physical pass.
///
/// The caller holds the affected parent `DIR` transaction guard(s); the sweep
/// runs strictly before the physical rmdir/rename (pre-commit), so a failure
/// aborts the removal and a retry converges.
pub(super) fn cleanup_upper_whiteouts(upper_dir_path: &Path) -> Result<()> {
    let mut names: Vec<String> = Vec::new();
    upper_dir_path.inode().readdir_at(0, &mut names)?;
    names.retain(|name| !path::is_dot_or_dotdot(name));
    // First pass — full validation: refuse the sweep before removing
    // anything if any physical child is not a whiteout.
    validate_whiteout_children(upper_dir_path, &names)?;
    // Second pass — re-check then unlink: each child is re-observed and
    // re-classified immediately before its removal.
    unlink_rechecked_whiteouts(upper_dir_path, &names)?;
    Ok(())
}

/// Returns whether the named physical child of `upper_dir_path` is a whiteout.
///
/// The child is re-observed through the base VFS `Path` layer (`lookup_child`)
/// and classified with the shared [`is_whiteout_inode`] predicate, keeping
/// the base view's `DentryChildren` coherent. Underlying lookup errors
/// propagate unchanged.
fn is_whiteout_child(upper_dir_path: &Path, name: &str) -> Result<bool> {
    let child_path = Path::new(
        upper_dir_path.mount_node().clone(),
        upper_dir_path
            .dentry()
            .as_dir_dentry_or_err()?
            .lookup_child(name)?,
    );
    is_whiteout_inode(child_path.inode())
}

/// Validates that every named physical child of `upper_dir_path` is a
/// whiteout, removing nothing.
///
/// The full-validation pass of the sweep: any non-whiteout child returns
/// `ENOTEMPTY`, so the sweep refuses the removal before any entry is deleted.
fn validate_whiteout_children(upper_dir_path: &Path, names: &[String]) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
    }
    Ok(())
}

/// Re-observes and unlinks every named whiteout child of `upper_dir_path`.
///
/// The removal pass of the sweep: each child is re-classified immediately
/// before its `unlink`, so an entry swapped in since the validation pass is
/// refused (`ENOTEMPTY`) instead of deleted. The re-check narrows but cannot
/// close the residual check-to-use window, so the upper directory must not be
/// modified concurrently.
fn unlink_rechecked_whiteouts(upper_dir_path: &Path, names: &[String]) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
        upper_dir_path.unlink(name)?;
    }
    Ok(())
}
