// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Dev/ino identity projection of the overlay namespace.
//!
//! This module owns the immutable per-mount [`IdentityPolicy`] (mounted as
//! `OverlayFs::identity`) and the published [`ObjectId`] value.
//!
//! The **xino matrix** decides, for one layer and one real inode, which
//! `st_dev`/`st_ino` pair the overlay publishes:
//!
//! - **same-fs passthrough** — every layer shares one underlying
//!   filesystem, so `st_ino` matches the underlying inode and `st_dev` is
//!   uniform;
//! - **xino effective** — the overlay publishes its own `st_dev` and an
//!   encoded `st_ino` (layer `fsid` in the high `xino_shift` bits, real
//!   ino in the payload);
//! - **xino off** — directories report the overlay `st_dev` plus a
//!   saturating allocated ino; non-directories report the underlying
//!   dev/ino; an ino that does not fit the xino payload falls back to the
//!   xino-off behavior (explicit, never silently wrong).
//!
//! A **lower-id record** is the durable `(container_dev_id,
//! lower_layer_root_ino, real_ino)` provenance that copy-up persists on the
//! upper inode. [`IdentityPolicy::project_object_id_from_lower_id`] feeds
//! such a record back through the same xino matrix so the object keeps a
//! constant `st_ino` across copy-up (authority-continuity); the record's
//! device/root pair is resolved to a per-mount `fsid` from the immutable
//! lower-layer snapshot. This is an additional input to the matrix, not a
//! replacement of `RealObjectKey`.
//!
//! # Structure
//!
//! | Item | Owns |
//! |---|---|
//! | [`IdentityPolicy`] | The immutable per-mount projection policy. |
//! | [`ObjectId`] | The published `st_dev`/`st_ino` of one object. |
//! | [`LowerLayerIdentity`] | One published layer's identity triplet. |
//!
//! # References
//!
//! - Overlayfs (Linux overlay filesystem, incl. the xino inode mapping):
//!   <https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html>

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;

use super::{lookup::RealObject, lower_id::LowerIdRecord};
use crate::{
    fs::fs_impls::overlayfs::{inode::ObjectFacts, mount::XinoMode},
    prelude::*,
};

/// Total bit width of a `u64` inode value.
const U64_BITS: u32 = u64::BITS;

/// The published `st_dev`/`st_ino` identity of one overlay object.
///
/// The pair is precomputed once by [`IdentityPolicy`] at inode creation and
/// stored on the `OverlayInode`; it identifies one logical overlay object
/// wherever that object is reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) struct ObjectId {
    /// Published `st_dev`.
    pub(in overlayfs) dev: DeviceId,
    /// Published `st_ino`.
    pub(in overlayfs) ino: u64,
}

/// One published layer's identity triplet.
#[derive(Clone, Copy, Debug)]
pub(in overlayfs) struct LowerLayerIdentity {
    /// The per-mount layer ordinal.
    pub(in overlayfs) fsid: u64,
    /// The backend container device id of the layer.
    pub(in overlayfs) container_dev_id: DeviceId,
    /// The layer root's real inode number.
    pub(in overlayfs) lower_layer_root_ino: u64,
}

/// The immutable per-mount dev/ino projection policy.
///
/// Invariants: `xino_shift <= 63` (enforced by [`IdentityPolicy::new`]);
/// `fallback_ino_allocator` never wraps (saturating); `is_all_layers_same_fs`
/// is fixed at construction; `lower_layer_devs` is an fsid-sorted immutable
/// snapshot with one entry per configured lower — never re-probed at runtime.
#[derive(Debug)]
pub(in overlayfs) struct IdentityPolicy {
    /// The `xino=` mode; consumed from the mount policy at construction.
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
    /// The xino layer id occupies the high 16 bits; the inode payload is the
    /// low 48 bits.
    pub(in overlayfs) const XINO_SHIFT: u32 = 16;

    /// Constructs the immutable projection policy from the published layer
    /// snapshot.
    ///
    /// The policy keeps a LOWER-only identity snapshot: the published layer
    /// list minus the upper's entry. Exclusion is by position, not by
    /// value — an upper sharing an underlying filesystem with a lower must
    /// keep the lower's entry so no lower is dropped only because its
    /// device id matches the upper's.
    pub(in overlayfs) fn new(
        overlay_dev_id: DeviceId,
        layer_devs: &[LowerLayerIdentity],
        upper_layer_dev_index: Option<usize>,
        xino_shift: u32,
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

    pub(in overlayfs) fn is_xino_effective(&self) -> bool {
        if self.is_all_layers_same_fs {
            return false;
        }
        matches!(self.xino_mode, XinoMode::Auto | XinoMode::On)
    }

    pub(in overlayfs) fn is_all_layers_same_fs(&self) -> bool {
        self.is_all_layers_same_fs
    }

    /// Projects the dev/ino identity of a real object from its own layer
    /// evidence.
    ///
    /// This is the entry for callers that already hold a [`RealObject`]
    /// (its `fsid`, real ino, and container dev are read directly from the
    /// object), as opposed to the lower-id entry that starts from a durable
    /// record, so callers need not unpack the three `RealObject` fields into
    /// `project` themselves.
    pub(in overlayfs) fn project_object_id(
        &self,
        real: &RealObject,
        is_directory: bool,
    ) -> ObjectId {
        self.project(
            real.fsid(),
            real.real_inode().ino(),
            real.container_dev_id(),
            is_directory,
        )
    }

    /// Projects the dev/ino identity from the durable lower-id record
    /// through the shared [`IdentityPolicy::project`] matrix; an unresolved
    /// origin pair leaves the caller on the visible-source fallback.
    pub(in overlayfs) fn project_object_id_from_lower_id(
        &self,
        lower_id: &LowerIdRecord,
        is_directory: bool,
    ) -> Option<ObjectId> {
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

    /// Projects one `(layer_id, real_ino, origin_dev)` identity input to
    /// its published `st_dev`/`st_ino`.
    ///
    /// The branches make every published `st_ino` unambiguous: shared-fs
    /// passes the origin through, an encodable pair is packed into the xino
    /// payload, and an unencodable pair takes the fallback. The fit test
    /// rejects truncation that would alias two layers; checked arithmetic
    /// keeps the `payload_bits == U64_BITS` case from shifting by the full width.
    fn project(
        &self,
        layer_id: u64,
        real_ino: u64,
        origin_dev: DeviceId,
        is_directory: bool,
    ) -> ObjectId {
        if self.is_all_layers_same_fs {
            return ObjectId {
                dev: origin_dev,
                ino: real_ino,
            };
        }
        // Xino encoding applies when both the real ino and the layer id
        // fit the encoded space.
        if self.is_xino_effective() && self.xino_fits(layer_id, real_ino) {
            let payload_bits = U64_BITS - self.xino_shift;
            let encoded_ino = if payload_bits == U64_BITS {
                real_ino
            } else {
                (layer_id << payload_bits) | real_ino
            };
            return ObjectId {
                dev: self.overlay_dev_id,
                ino: encoded_ino,
            };
        }
        // No xino encoding (or the ino overflowed the payload):
        // directories take the overlay dev plus an allocated ino, so they
        // stay stable without an encodable payload; non-directories pass
        // through the origin dev/ino unchanged.
        if is_directory {
            ObjectId {
                dev: self.overlay_dev_id,
                ino: self.allocate_fallback_ino(),
            }
        } else {
            ObjectId {
                dev: origin_dev,
                ino: real_ino,
            }
        }
    }

    /// Returns whether the `(layer_id, real_ino)` pair fits the xino-encoded
    /// ino space.
    ///
    /// The same fit predicate gates the xino encode and the directory
    /// determinism check, so the readdir `..` route can decide before
    /// projecting whether its published `d_ino("..")` stays stable across
    /// calls. Checked arithmetic skips the degenerate `payload_bits == U64_BITS`
    /// (`xino_shift == 0`) case, so it never shifts by the full bit width.
    fn xino_fits(&self, layer_id: u64, real_ino: u64) -> bool {
        let payload_bits = U64_BITS - self.xino_shift;
        payload_bits == U64_BITS
            || (real_ino >> payload_bits == 0 && layer_id >> self.xino_shift == 0)
    }

    /// Returns whether projecting the `(layer_id, real_ino)` pair as a
    /// directory is deterministic — same-fs passthrough or a fitting xino
    /// encode — rather than the xino-off directory branch that allocates a
    /// fresh fallback ino per call.
    pub(in overlayfs) fn is_directory_projection_deterministic(
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
    /// Saturating by construction (the counter never wraps). The first
    /// allocation returns `1` (ino 0 is not handed out).
    fn allocate_fallback_ino(&self) -> u64 {
        match self.fallback_ino_allocator.try_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(1)),
        ) {
            Ok(previous) => previous.saturating_add(1),
            Err(_) => u64::MAX,
        }
    }

    /// Resolves the unique per-mount layer id (fsid) for a durable origin
    /// record's `(container_dev_id, lower_layer_root_ino)` pair among the
    /// current lower layers.
    ///
    /// The LOWER-only table is consulted because origin records only ever
    /// come from lower sources; an absent pair or multiple matching fsids
    /// returns `None` to keep the visible-source fallback.
    pub(in overlayfs) fn resolve_layer_id_for_record(
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

    /// Returns whether the persisted lower-source record is consistent with
    /// the retained same-layer lower of `facts`: the record's real inode must
    /// equal the retained lower's, so a forged or stale record falls back to
    /// the visible-source projection (identity authenticity wins).
    ///
    // TODO(origin-verify): once the VFS gains an ino-to-inode / file-handle
    // resolution surface, upgrade this cross-check to a full origin
    // verification and drop the retained-lower approximation.
    pub(super) fn origin_real_ino_resolves(
        &self,
        record: &LowerIdRecord,
        facts: &ObjectFacts,
    ) -> bool {
        // The record's layer is the unique current lower fsid matching its
        // device/root pair; the retained lower at that layer is the
        // same-layer evidence in the fresh facts.
        let Some(layer_fsid) = self
            .resolve_layer_id_for_record(record.container_dev_id(), record.lower_layer_root_ino())
        else {
            return false;
        };
        match facts.lowers.iter().find(|lower| lower.fsid() == layer_fsid) {
            // Accepted only when the record's real inode equals the retained
            // same-layer lower; a mismatch (the lower was replaced since
            // copy-up) or an absent retained lower (the lower no longer
            // participates in the name) rejects the record.
            Some(retained_lower) => record.real_ino() == retained_lower.real_inode().ino(),
            None => false,
        }
    }
}
