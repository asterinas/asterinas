// SPDX-License-Identifier: MPL-2.0

//! Dev/ino identity projection of the overlay namespace.
//!
//! This module owns the immutable per-mount [`IdentityPolicy`] (mounted as
//! `OverlayFs::identity`) and the published [`OverlayObjectId`] value. It
//! implements the dev/ino projection matrix:
//!
//! - **same-fs passthrough** — when every layer shares one underlying
//!   filesystem, `st_dev` is uniform and `st_ino` matches the underlying
//!   inode (fast path);
//! - **xino effective** — overlay `st_dev` plus an encoded `st_ino` (the
//!   layer `fsid` in the high `xino_shift` bits, real ino in the payload);
//! - **xino off** — directories report the overlay `st_dev` plus a saturating
//!   allocated ino; non-directories report the underlying dev/ino;
//! - **per-object overflow** — an ino that does not fit the xino payload
//!   falls back to the xino-off behavior (explicit fallback, never silently
//!   wrong).
//!
//! The lower-id consumption path
//! [`IdentityPolicy::project_object_id_from_lower_id`] projects a durable
//! lower-id record through the SAME matrix with the record's
//! `(container_dev_id, lower_layer_root_ino, real_ino)` as the identity input
//! — constant `st_ino` across copy-up (authority-continuity invariant). It
//! binds the durable device to the configured lower root and resolves the
//! pair to a per-mount `fsid` from the immutable lower-layer snapshot. It is
//! a new input to the existing projection, never a replacement of
//! `RealObjectKey`/the xino matrix.
//!
//! # Locking
//!
//! [`IdentityPolicy`] is immutable policy inside `OverlayFs::identity`; the
//! only mutable state is the genuinely independent saturating
//! `fallback_ino_allocator` counter. Construction consumes the all-layer
//! input and retains only the immutable lower snapshot; neither is runtime
//! state or a lock. The projection functions are pure, lock-free transforms;
//! they are called from inode creation under the caller's `DIR` transaction
//! (or lock-free at stat time) and hold no Overlay lock.

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;

use super::{entry::RealObject, lower_id::LowerIdRecord};
// `XinoMode` is declared in `mount/options.rs` and consumed here.
use crate::{fs::fs_impls::overlayfs::mount::XinoMode, prelude::*};

/// The published `st_dev`/`st_ino` identity of one overlay object.
///
/// The pair is precomputed once by [`IdentityPolicy`] at inode creation and
/// stored on the `OverlayInode`; stat reuses it without re-derivation. It is
/// an identity projection, never a reverse name map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) struct OverlayObjectId {
    /// Published `st_dev`.
    pub(in crate::fs::fs_impls::overlayfs) dev: DeviceId,
    /// Published `st_ino`.
    pub(in crate::fs::fs_impls::overlayfs) ino: u64,
}

/// One published layer's identity triplet — the named record of the
/// construction-local `layer_devs` input of [`IdentityPolicy::new`].
///
/// `fsid` is the per-mount layer ordinal, `container_dev_id` the backend
/// device id, and `lower_layer_root_ino` the layer root's real inode number.
#[derive(Clone, Copy, Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct LowerLayerIdentity {
    /// The per-mount layer ordinal.
    pub(in crate::fs::fs_impls::overlayfs) fsid: u64,
    /// The backend container device id of the layer.
    pub(in crate::fs::fs_impls::overlayfs) container_dev_id: DeviceId,
    /// The layer root's real inode number.
    pub(in crate::fs::fs_impls::overlayfs) lower_layer_root_ino: u64,
}

/// The immutable per-mount dev/ino projection policy.
///
/// Invariants: `xino_shift <= 63` (enforced by [`IdentityPolicy::new`]);
/// `fallback_ino_allocator` never wraps (saturating, see
/// [`IdentityPolicy::allocate_fallback_ino`]); `is_all_layers_same_fs` is
/// fixed at construction; `lower_layer_devs` is an fsid-sorted immutable
/// snapshot with one entry per configured lower — never re-probed at runtime.
/// Storage: immutable policy inside `OverlayFs::identity`; the allocator
/// is a genuinely independent counter; the construction-local all-layer input
/// is discarded after deriving this state.
#[derive(Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct IdentityPolicy {
    /// The `xino=` mode; consumed from the mount policy at construction
    /// (`mount/build.rs` passes `policy.xino_mode()`).
    xino_mode: XinoMode,
    /// The overlay's own `st_dev` (`AnonDeviceId`), acquired in the extended
    /// `OverlayFs::new`.
    overlay_dev_id: DeviceId,
    /// High-bit encoding width of the xino layer id (e.g. `64 - 16` = 48-bit
    /// payload).
    xino_shift: u32,
    /// Whether every layer shares one underlying filesystem (fast path);
    /// derived at construction from the published layer dev ids.
    is_all_layers_same_fs: bool,
    /// Immutable LOWER-only identity snapshot for durable origin records.
    lower_layer_devs: Box<[LowerLayerIdentity]>,
    /// Saturating fallback ino allocator for directories / anon inos when
    /// xino is not applicable.
    fallback_ino_allocator: AtomicU64,
}

impl IdentityPolicy {
    /// Constructs the immutable projection policy from the published layer
    /// snapshot.
    ///
    /// `overlay_dev_id` is the overlay `AnonDeviceId` acquired in the extended
    /// `OverlayFs::new`; the construction-local `layer_devs` input carries one
    /// [`LowerLayerIdentity`] per published layer, from the same snapshot that
    /// feeds `is_all_layers_same_fs`. Only the derived same-fs state and
    /// fsid-sorted lower snapshot survive construction.
    /// `upper_layer_dev_index` is the position of the upper's entry in
    /// `layer_devs` (the builder pushes the upper first when present; `None`
    /// on a read-only mount): the LOWER-only view is derived by excluding
    /// exactly that entry, so origin-record pair resolution never lets the
    /// upper's entry participate. The exclusion is by position, not by value
    /// — an upper sharing an underlying filesystem with a lower must keep the
    /// lower's entry. `xino_mode` is consumed from the mount policy at
    /// construction so the policy stays immutable and no `Weak<OverlayFs>`
    /// back-reference is added. The invariant `xino_shift <= 63` is enforced
    /// at construction: a violating shift is a mount-policy programming error
    /// and is rejected instead of building a broken policy.
    ///
    /// Constructed once in the extended `OverlayFs::new` (`mount/build.rs`).
    pub(in crate::fs::fs_impls::overlayfs) fn new(
        overlay_dev_id: DeviceId,
        layer_devs: &[LowerLayerIdentity],
        upper_layer_dev_index: Option<usize>,
        xino_shift: u32,
        // `xino_mode` is the parsed `xino=` option value.
        xino_mode: XinoMode,
    ) -> Result<Self> {
        if xino_shift > 63 {
            return_errno_with_message!(Errno::EINVAL, "invalid overlay xino shift");
        }
        let is_all_layers_same_fs = layer_devs.first().is_some_and(|first| {
            layer_devs
                .iter()
                .all(|layer| layer.container_dev_id == first.container_dev_id)
        });
        let mut lower_layer_devs: Vec<LowerLayerIdentity> = layer_devs
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| Some(*index) != upper_layer_dev_index)
            .map(|(_, layer)| layer)
            .collect();
        lower_layer_devs.sort_by_key(|layer| layer.fsid);
        Ok(Self {
            xino_mode,
            overlay_dev_id,
            xino_shift,
            is_all_layers_same_fs,
            lower_layer_devs: lower_layer_devs.into_boxed_slice(),
            fallback_ino_allocator: AtomicU64::new(0),
        })
    }

    /// Returns whether the xino encoding branch of the projection matrix
    /// applies.
    ///
    /// Same-fs passthrough takes precedence: when every layer shares one
    /// underlying filesystem, raw underlying dev/ino is already uniform, so
    /// xino is not effective (the matrix's first branch is selected
    /// before this check at the call sites). Otherwise `Auto`/`On` is
    /// effective and `Off` is not. Feasibility for `Auto` gates on every
    /// underlying filesystem providing persistent inode identity — Asterinas
    /// has no export-style FH surface, so no feasibility probe exists and
    /// `Auto` is treated as feasible.
    pub(in crate::fs::fs_impls::overlayfs) fn is_xino_effective(&self) -> bool {
        // Matrix branch 1 precedence: same-fs passthrough wins first.
        if self.is_all_layers_same_fs {
            return false;
        }
        // xino-mode table:
        //   Off  -> false            (encoded-ino branch disabled; branches 3/4)
        //   Auto -> true             (feasible-by-default: no export-FH probe exists)
        //   On   -> true             (forced; per-object overflow still falls
        //                             back to branch 4)
        matches!(self.xino_mode, XinoMode::Auto | XinoMode::On)
    }

    /// Returns whether every layer shares one underlying filesystem (fast
    /// path; the matrix's branch-1 predicate).
    ///
    /// Derived at construction from the published layer dev ids and
    /// immutable thereafter. Consumed by the sibling `readdir_index.rs`
    /// through `OverlayFs::identity()` — the `..` route short-circuits on a
    /// multi-fs xino-off mount, and the record arm skips the layer-id
    /// resolution on an all-same-fs stack (branch 1 needs no layer id).
    pub(in crate::fs::fs_impls::overlayfs) fn is_all_layers_same_fs(&self) -> bool {
        self.is_all_layers_same_fs
    }

    /// Projects the dev/ino identity of the visible-metadata source.
    ///
    /// The projection matrix is implemented once in the shared
    /// [`IdentityPolicy::project`] helper; this method supplies the visible
    /// source's `(fsid, real ino, origin dev)` and delegates.
    pub(in crate::fs::fs_impls::overlayfs) fn project_object_id(
        &self,
        real: &RealObject,
        is_directory: bool,
    ) -> OverlayObjectId {
        self.project(
            real.fsid(),
            real.real_inode().ino(),
            real.container_dev_id(),
            is_directory,
        )
    }

    /// Projects the dev/ino identity from the durable lower-id record.
    ///
    /// The SAME projection matrix as [`IdentityPolicy::project_object_id`]
    /// (one shared [`IdentityPolicy::project`] helper) with the record's
    /// `(container_dev_id, lower_layer_root_ino, real_ino)` as the identity
    /// input. It resolves the origin pair before selecting any matrix branch,
    /// so same-fs and xino-off projections validate foreign or ambiguous
    /// evidence too. [`IdentityPolicy::resolve_layer_id_for_record`] returns
    /// the unique current lower `fsid`; `None` when the pair does not resolve
    /// uniquely leaves the caller on the visible-source fallback (never
    /// silently wrong).
    pub(in crate::fs::fs_impls::overlayfs) fn project_object_id_from_lower_id(
        &self,
        lower_id: &LowerIdRecord,
        is_directory: bool,
    ) -> Option<OverlayObjectId> {
        let layer_id = self.resolve_layer_id_for_record(
            lower_id.container_dev_id(),
            lower_id.lower_layer_root_ino(),
        )?;
        Some(self.project(
            layer_id,
            lower_id.real_ino(),
            lower_id.container_dev_id(),
            is_directory,
        ))
    }

    /// Runs the four-branch dev/ino projection matrix for one `(layer_id,
    /// real_ino, origin_dev)` identity input.
    ///
    /// The identical matrix is executed by both
    /// [`IdentityPolicy::project_object_id`] and
    /// [`IdentityPolicy::project_object_id_from_lower_id`] (two call paths
    /// inside this module), so the branches — including the fit tests — live
    /// in exactly one place.
    ///
    /// Matrix: **1** same-fs passthrough (`origin_dev` + `real_ino`);
    /// **2** xino effective → overlay `st_dev` plus an encoded `st_ino` (the
    /// layer id in the high `xino_shift` bits, real ino in the payload); **3**
    /// xino off → directories get the overlay dev plus an allocated ino,
    /// non-directories report `origin_dev`/`real_ino`; **4** per-object
    /// overflow (the real ino does not fit the payload, or the layer id does
    /// not fit the `xino_shift`-bit layer-id space) → the explicit xino-off
    /// fallback. The layer-id fit test closes the silent-truncation hole:
    /// without it, two layer ids differing only above bit `xino_shift` would
    /// encode to the same published `st_ino`. Uses checked arithmetic: the
    /// `payload_bits == 64` short-circuit skips the degenerate
    /// `xino_shift == 0` case and never shifts by the full bit width.
    fn project(
        &self,
        layer_id: u64,
        real_ino: u64,
        origin_dev: DeviceId,
        is_directory: bool,
    ) -> OverlayObjectId {
        // Matrix branch 1: same-fs passthrough. All layers share one
        // underlying filesystem, so the origin layer's device is the shared
        // underlying dev and the real ino is already uniform.
        if self.is_all_layers_same_fs {
            return OverlayObjectId {
                dev: origin_dev,
                ino: real_ino,
            };
        }
        // Matrix branch 2: xino effective with a fitting real ino AND
        // a fitting layer id (the layer id must fit the `xino_shift`-bit
        // layer-id space; higher bits would be silently dropped by the
        // encode — the fit test is the shared
        // [`IdentityPolicy::xino_fits`] helper, used by the branch-2 encode
        // and the readdir `..` determinism gate).
        if self.is_xino_effective() && self.xino_fits(layer_id, real_ino) {
            let payload_bits = 64 - self.xino_shift;
            let encoded_ino = if payload_bits == 64 {
                real_ino
            } else {
                (layer_id << payload_bits) | real_ino
            };
            return OverlayObjectId {
                dev: self.overlay_dev_id,
                ino: encoded_ino,
            };
        }
        // Per-object overflow (real ino and/or layer id does not fit) or
        // xino off: fall through to the explicit fallback below.
        // Matrix branches 3/4: xino off (or per-object overflow fallback):
        // dirs get the overlay dev + an allocated ino; non-dirs report the
        // origin dev/ino.
        if is_directory {
            OverlayObjectId {
                dev: self.overlay_dev_id,
                ino: self.allocate_fallback_ino(),
            }
        } else {
            OverlayObjectId {
                dev: origin_dev,
                ino: real_ino,
            }
        }
    }

    /// Returns whether the `(layer_id, real_ino)` pair fits the xino-encoded
    /// ino space (matrix branch-2 precondition; the fit test is extracted so
    /// the branch-2 encode and the determinism gate below share one
    /// implementation — the readdir `..` route gates on it before projecting,
    /// so `d_ino("..")` stays stable across calls). Checked arithmetic: the
    /// `payload_bits == 64` short-circuit skips the degenerate
    /// `xino_shift == 0` case and never shifts by the full bit width.
    fn xino_fits(&self, layer_id: u64, real_ino: u64) -> bool {
        let payload_bits = 64 - self.xino_shift;
        payload_bits == 64 || (real_ino >> payload_bits == 0 && layer_id >> self.xino_shift == 0)
    }

    /// Returns whether projecting the `(layer_id, real_ino)` pair as a
    /// directory is deterministic — same-fs passthrough (branch 1) or a
    /// fitting xino encode (branch 2) — i.e. the matrix does NOT take
    /// the xino-off/overflow directory branch that allocates a fresh fallback
    /// ino per call (the readdir `..` route gates on this before projecting,
    /// so `d_ino("..")` stays stable across calls). Consumed by the sibling
    /// `readdir_index.rs` through `OverlayFs::identity()`.
    pub(in crate::fs::fs_impls::overlayfs) fn is_directory_projection_deterministic(
        &self,
        layer_id: u64,
        real_ino: u64,
    ) -> bool {
        if self.is_all_layers_same_fs {
            return true;
        }
        self.is_xino_effective() && self.xino_fits(layer_id, real_ino)
    }

    /// Allocates a fallback ino for directories / anon objects when xino is
    /// not applicable.
    ///
    /// Saturating by construction (the counter never wraps):
    /// [`AtomicU64::try_update`] commits `saturating_add(1)` and retries on
    /// contention, so the committed counter converges to and stays at
    /// `u64::MAX`; the returned value is the newly committed counter. The
    /// first allocation returns `1` (ino 0 is not handed out).
    fn allocate_fallback_ino(&self) -> u64 {
        match self.fallback_ino_allocator.try_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(1)),
        ) {
            // The closure never returns `None`, so `try_update` always
            // succeeds; this arm is defensive and unreachable.
            Ok(previous) => previous.saturating_add(1),
            Err(_) => u64::MAX,
        }
    }

    /// Resolves the unique per-mount layer id (fsid) for a durable origin
    /// record's `(container_dev_id, lower_layer_root_ino)` pair among the
    /// CURRENT LOWER layers.
    ///
    /// The LOWER-only `lower_layer_devs` table is consulted: origin records
    /// only ever come from lower sources, so the upper's entry never
    /// participates — an upper sharing `st_dev` with a lower must not make a
    /// valid record read as ambiguous. Returns the unique fsid when exactly
    /// one DISTINCT lower fsid matches the pair; repeated matching entries
    /// with that same fsid remain usable. An absent pair or multiple matching
    /// fsids returns `None`, conservatively preserving the visible-source
    /// fallback rather than attributing a record to the wrong layer. Consumed
    /// by the sibling `readdir_index.rs` record arm and by
    /// [`IdentityPolicy::project_object_id_from_lower_id`] through
    /// `OverlayFs::identity()`.
    pub(in crate::fs::fs_impls::overlayfs) fn resolve_layer_id_for_record(
        &self,
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
    ) -> Option<u64> {
        let mut matched_fsid: Option<u64> = None;
        for layer in self.lower_layer_devs.iter() {
            if layer.container_dev_id == container_dev_id
                && layer.lower_layer_root_ino == lower_layer_root_ino
            {
                match matched_fsid {
                    None => matched_fsid = Some(layer.fsid),
                    Some(existing) if existing == layer.fsid => {}
                    Some(_) => return None,
                }
            }
        }
        matched_fsid
    }
}
