// SPDX-License-Identifier: MPL-2.0

//! Real-object projection and the upper-first layer lookup core.
//!
//! This module owns [`RealObject`] — the pinned real (underlying) object of one
//! layer — its private whiteout/opaque marker reads, and the module-private
//! [`LayerLookup`] intermediate produced by [`OverlayFs::lookup_in_layers`].
//! The lookup scan is upper-first and matches the Linux `ovl_lookup_single`
//! merge-stop semantics (verified against the Linux source tree
//! `fs/overlayfs/namei.c`, function `ovl_lookup_single`): the first
//! non-directory hit terminates as `Single`; directory hits accumulate into
//! the lower stack until a barrier — a whiteout (negative), an opaque
//! directory found at the name (the merge stops below it, namei.c:324-331),
//! or a non-directory below an accumulated directory (the merge stops,
//! namei.c:298-299) — or the upper-miss opaque-parent case (negative).

use device_id::DeviceId;

use super::{
    binding_cache::{HiddenEvidence, NegativeBinding, PositiveKind},
    inode::OverlayObjectFacts,
};
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::mount::{OverlayFs, RealPath},
        vfs::{inode::Inode, path::Path, xattr::XattrName},
    },
    prelude::*,
};

/// The xattr name of the xattr-based whiteout marker (Linux `OVL_XATTR_XWHITEOUT`).
const WHITEOUT_XATTR_FULL_NAME: &str = "trusted.overlay.whiteout";

/// The xattr name of the opaque-directory marker (Linux `OVL_XATTR_OPAQUE`).
const OPAQUE_XATTR_FULL_NAME: &str = "trusted.overlay.opaque";

/// Returns whether `real_inode` is a whiteout.
///
/// The single whiteout predicate of the overlayfs tree (Linux
/// `ovl_is_whiteout`): a whiteout is either a classic character device `0:0`
/// or an object carrying the `trusted.overlay.whiteout` marker. The marker
/// read is value-based: a 1-byte value of exactly `b'y'` proves the whiteout
/// marker; an absent (`ENODATA`) or unsupported (`EOPNOTSUPP`) marker, any
/// non-canonical value, or a value longer than the 1-byte probe (`ERANGE`)
/// reads as "not a whiteout" — matching the opaque-directory predicate and
/// Linux `ovl_check_xwhiteout`. Genuine xattr errors propagate.
pub(in crate::fs::fs_impls::overlayfs) fn is_whiteout_inode(
    real_inode: &Arc<dyn Inode>,
) -> Result<bool> {
    let metadata = real_inode.metadata()?;
    // A classic whiteout is a character device with device number 0:0.
    // Backends report that device number either as
    // `Some(DeviceId::null())` or — when the device number is zero
    // (e.g. ramfs) — as `None`.
    if metadata.type_ == InodeType::CharDevice
        && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
    {
        return Ok(true);
    }
    let name = XattrName::try_from_full_name(WHITEOUT_XATTR_FULL_NAME).ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "invalid overlay whiteout marker xattr name")
    })?;
    let mut value = [0u8; 1];
    let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    match real_inode.get_xattr(name, &mut writer) {
        Ok(written) => Ok(written == 1 && value[0] == b'y'),
        Err(err) if err.error() == Errno::ERANGE => Ok(false),
        Err(err) if err.error() == Errno::ENODATA || err.error() == Errno::EOPNOTSUPP => Ok(false),
        Err(err) => Err(err),
    }
}

/// One pinned real (underlying) object of an overlay layer.
///
/// `layer_index` is the object's position in the overlay layer stack (`0` =
/// upper, `1..` = lower position); `fsid` is the per-unique-underlying-
/// superblock identifier; `container_dev_id` is the `st_dev` evidence of the
/// same layer. The real inode is a strong pin: `RealObject` values inside
/// facts are immutable while published — facts are replaced, never mutated in
/// place.
///
/// Invariants: the pin is strong; the fields are fixed for the lifetime of the
/// value. The dentry-anchored [`RealPath`] (`real_path`) is the
/// base-view coherence anchor: it is `Some` for every real object that
/// participates in a namespace mutation or dentry-routed lookup (upper
/// objects always, lower objects produced by the layer scan, root objects
/// via the layer anchor) and `None` only for the readdir `..` identity
/// projection, which never mutates — enforced by the checked
/// [`RealObject::real_path`] accessor (`Err(EIO)` on `None`). The named
/// constructor ([`RealObject::new`]) is retained for that
/// single identity-only producer (`readdir_index.rs`); within-`projection`
/// builders (root facts in `inode.rs`, the lookup scan here,
/// `RealObjectKey::from_facts`) construct through the `pub(super)` fields
/// directly or through [`RealObject::with_path`], so there is a single
/// construction path for sibling modules and no field widening beyond the
/// projection tree.
///
/// The leaf modules (`readdir_index.rs`, `copyup/`, `dir/`) name the layer
/// real objects.
#[derive(Clone, Debug)]
pub(in crate::fs::fs_impls::overlayfs) struct RealObject {
    pub(super) layer_index: usize,
    pub(super) real_inode: Arc<dyn Inode>,
    /// Dentry-anchored real-object [`RealPath`] value; `None` only for the
    /// readdir `..` identity projection (see the struct doc).
    pub(super) real_path: Option<RealPath>,
    pub(super) fsid: u64,
    pub(super) container_dev_id: DeviceId,
}

impl RealObject {
    /// Builds a real object from its four fields.
    ///
    /// The path-less constructor is **identity-only**: its single producer is
    /// the readdir `..` identity projection (`readdir_index.rs`), which never
    /// mutates and never participates in dentry-routed lookup. Every other
    /// real-object construction carries the dentry-anchored [`RealPath`] via
    /// [`RealObject::with_path`] or a `pub(super)` field literal with
    /// `real_path: Some(..)`.
    pub(in crate::fs::fs_impls::overlayfs) fn new(
        layer_index: usize,
        real_inode: Arc<dyn Inode>,
        fsid: u64,
        container_dev_id: DeviceId,
    ) -> Self {
        Self {
            layer_index,
            real_inode,
            real_path: None,
            fsid,
            container_dev_id,
        }
    }

    /// Builds a dentry-anchored real object from its layer position and the
    /// real object's dentry-anchored [`RealPath`].
    ///
    /// Derives the pinned `real_inode` from the path
    /// (`real_path.inode()`), so the path's inode and dentry always refer to
    /// the same dentry-layer object. The path keeps the base-view dentry
    /// layer coherent for every namespace-mutating or dentry-routed consumer; its
    /// anchor mount is held weakly, so the published object never pins the
    /// parent overlay's `Mount`/`OverlayFs` lifetime.
    pub(in crate::fs::fs_impls::overlayfs) fn with_path(
        layer_index: usize,
        real_path: RealPath,
        fsid: u64,
        container_dev_id: DeviceId,
    ) -> Self {
        Self {
            layer_index,
            real_inode: real_path.inode().clone(),
            real_path: Some(real_path),
            fsid,
            container_dev_id,
        }
    }

    /// Builds the dentry-anchored real object for one layer hit of a child
    /// lookup inside the layer scan.
    ///
    /// The child hit inherits the parent layer's `fsid` and
    /// `container_dev_id` evidence and pins the dentry-anchored `child_path`;
    /// the pinned real inode is derived from that path. Shared by
    /// the upper and lower arms of `lookup_in_layers` so the two hit
    /// constructions cannot drift.
    fn for_lookup_child(layer_index: usize, child_path: &Path, layer_real: &RealObject) -> Self {
        Self::with_path(
            layer_index,
            RealPath::from_path(child_path),
            layer_real.fsid(),
            layer_real.container_dev_id(),
        )
    }

    /// Returns the position of this real object in the overlay layer stack.
    pub(in crate::fs::fs_impls::overlayfs) fn layer_index(&self) -> usize {
        self.layer_index
    }

    /// Returns the pinned underlying inode of this real object.
    pub(in crate::fs::fs_impls::overlayfs) fn real_inode(&self) -> &Arc<dyn Inode> {
        &self.real_inode
    }

    /// Returns the dentry-anchored real-object `Path`, upgraded from the
    /// stored weak-anchor path.
    ///
    /// `Err(EIO)` when this real object carries no path — the readdir `..`
    /// identity projection is the only path-less producer, and no
    /// namespace-mutating or dentry-routed caller may operate on it — or
    /// when the anchor mount is no longer alive (the parent overlay was
    /// unmounted while this path survived; fail-closed, matching the
    /// existing "mount no longer alive" convention).
    pub(in crate::fs::fs_impls::overlayfs) fn real_path(&self) -> Result<Path> {
        self.real_path
            .as_ref()
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EIO,
                    "the real object carries no dentry-anchored path",
                )
            })?
            .upgrade()
    }

    /// Returns the layer filesystem identifier of this real object.
    pub(in crate::fs::fs_impls::overlayfs) fn fsid(&self) -> u64 {
        self.fsid
    }

    /// Returns the `st_dev` of the container filesystem of this real object.
    pub(in crate::fs::fs_impls::overlayfs) fn container_dev_id(&self) -> DeviceId {
        self.container_dev_id
    }

    /// Returns whether this real object is a whiteout.
    ///
    /// Delegates to the shared [`is_whiteout_inode`] predicate — the single
    /// source of truth for the whiteout test.
    fn is_whiteout(&self) -> Result<bool> {
        is_whiteout_inode(&self.real_inode)
    }

    /// Returns whether this real object is an opaque directory.
    ///
    /// An opaque directory carries `trusted.overlay.opaque == "y"` and acts
    /// as a lower-search barrier. Only directories qualify; the marker is
    /// re-observed on every lookup (no marker cache). Absent, unsupported, or
    /// over-long markers read as "not opaque"; genuine xattr errors propagate.
    pub(in crate::fs::fs_impls::overlayfs) fn is_opaque_directory(&self) -> Result<bool> {
        if !self.real_inode.type_().is_directory() {
            return Ok(false);
        }
        let name = XattrName::try_from_full_name(OPAQUE_XATTR_FULL_NAME).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "invalid overlay opaque marker xattr name")
        })?;
        let mut value = [0u8; 1];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match self.real_inode.get_xattr(name, &mut writer) {
            Ok(written) => Ok(written == 1 && value[0] == b'y'),
            Err(err)
                if err.error() == Errno::ENODATA
                    || err.error() == Errno::EOPNOTSUPP
                    || err.error() == Errno::ERANGE =>
            {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }
}

/// The module-private layer-lookup outcome (entry.rs) — the only named
/// intermediate of the lookup path.
///
/// The payloads are final types (`OverlayObjectFacts` / `NegativeBinding`);
/// `PositiveBinding` is assembled by the caller (`lookup_binding` in `mod.rs`)
/// after `project_inode` runs, which is why the positive payload is facts
/// rather than a binding.
pub(super) enum LayerLookup {
    Positive(OverlayObjectFacts),
    Negative(NegativeBinding),
}

impl OverlayFs {
    /// Runs the upper-first layer lookup for `name` inside `parent_facts`'s
    /// real layers.
    ///
    /// Scan contract, matching Linux `ovl_lookup_single` (verified
    /// against namei.c): layers are observed topmost-first
    /// (`parent_facts.upper`, then `parent_facts.lowers`); the first
    /// non-directory hit terminates as `Single`; directory hits accumulate
    /// into the lower stack; a whiteout hit terminates as `HiddenByWhiteout`;
    /// an opaque directory found at the name stops the downward merge at any
    /// layer (`val == 'y'` -> `d->stop`, namei.c:324-331); a non-directory
    /// below an accumulated directory stops the merge (namei.c:298-299); an
    /// opaque parent upper (re-observed `trusted.overlay.opaque == "y"`)
    /// terminates names absent from the upper as `HiddenByOpaque` without a
    /// lower scan. Every layer hit is resolved through the parent's
    /// dentry-anchored path (`Dentry::lookup_child` on the base VFS dentry
    /// layer, which revalidates cached entries and updates the base view's
    /// `DentryChildren`), and the hit keeps that dentry-anchored
    /// [`RealPath`]. The caller holds the parent `DIR` transaction lock;
    /// this function takes no Overlay lock itself.
    pub(super) fn lookup_in_layers(
        &self,
        parent_facts: &OverlayObjectFacts,
        name: &str,
    ) -> Result<LayerLookup> {
        // The accumulation of directory hits (topmost-first) for the merged
        // directory case; a raw local of the lookup, not a named type.
        let mut dir_hits: Vec<RealObject> = Vec::new();

        // Layer 0: the upper component of the parent, when present.
        if let Some(upper_real) = &parent_facts.upper {
            let upper_path = upper_real.real_path()?;
            match upper_path
                .dentry()
                .as_dir_dentry_or_err()?
                .lookup_child(name)
            {
                Ok(child_dentry) => {
                    let child_path = Path::new(upper_path.mount_node().clone(), child_dentry);
                    let hit = RealObject::for_lookup_child(0, &child_path, upper_real);
                    if hit.is_whiteout()? {
                        return Ok(LayerLookup::Negative(NegativeBinding::HiddenByWhiteout(
                            HiddenEvidence {
                                layer_index: 0,
                                real_inode: hit.real_inode().clone(),
                            },
                        )));
                    }
                    if !hit.real_inode().type_().is_directory() {
                        // The first non-directory hit terminates as `Single`
                        // and hides all lower hits.
                        return Ok(LayerLookup::Positive(OverlayObjectFacts {
                            kind: PositiveKind::Single,
                            upper: Some(hit),
                            lowers: Vec::new(),
                        }));
                    }
                    if hit.is_opaque_directory()? {
                        // An opaque directory found at the name is a merge
                        // barrier at EVERY layer, including the upper (Linux
                        // `ovl_lookup_single`: `val == 'y'` -> `d->stop =
                        // true`; namei.c:324-331): its lower counterparts are
                        // hidden, so the upper directory is the sole visible
                        // layer entry.
                        return Ok(LayerLookup::Positive(OverlayObjectFacts {
                            kind: PositiveKind::Single,
                            upper: Some(hit),
                            lowers: Vec::new(),
                        }));
                    }
                    dir_hits.push(hit);
                }
                Err(err) if err.error() == Errno::ENOENT => {
                    // The name is absent from the upper. An opaque upper
                    // directory is a lower-search barrier: the name is hidden
                    // and lower layers are never scanned.
                    if upper_real.is_opaque_directory()? {
                        return Ok(LayerLookup::Negative(NegativeBinding::HiddenByOpaque(
                            HiddenEvidence {
                                layer_index: 0,
                                real_inode: upper_real.real_inode().clone(),
                            },
                        )));
                    }
                }
                Err(err) => return Err(err),
            }
        }

        // Lower layers, topmost-first (layer positions `1..`). Each child hit
        // inherits the mount-layer position of the `lower_real` that produced
        // it: a dense `offset + 1` would only coincide when the retained
        // lowers start at layer 1 with no gaps.
        for lower_real in &parent_facts.lowers {
            let layer_index = lower_real.layer_index();
            let lower_path = lower_real.real_path()?;
            match lower_path
                .dentry()
                .as_dir_dentry_or_err()?
                .lookup_child(name)
            {
                Ok(child_dentry) => {
                    let child_path = Path::new(lower_path.mount_node().clone(), child_dentry);
                    let hit = RealObject::for_lookup_child(layer_index, &child_path, lower_real);
                    if hit.is_whiteout()? {
                        // A whiteout is the topmost occurrence of the name:
                        // the name is hidden. Below an already-visible
                        // directory it only ends the downward merge scan.
                        if dir_hits.is_empty() {
                            return Ok(LayerLookup::Negative(NegativeBinding::HiddenByWhiteout(
                                HiddenEvidence {
                                    layer_index,
                                    real_inode: hit.real_inode().clone(),
                                },
                            )));
                        }
                        break;
                    }
                    if !hit.real_inode().type_().is_directory() {
                        if dir_hits.is_empty() {
                            // The first non-directory hit terminates as
                            // `Single`; lower hits are hidden.
                            return Ok(LayerLookup::Positive(OverlayObjectFacts {
                                kind: PositiveKind::Single,
                                upper: None,
                                lowers: vec![hit],
                            }));
                        }
                        // A non-directory below an accumulated directory hit
                        // stops the downward merge: every deeper layer stays
                        // hidden (Linux `ovl_lookup_single`:
                        // `!d_can_lookup(this)` with `d->is_dir` already set
                        // -> `d->stop = true`; namei.c:298-299).
                        break;
                    }
                    let is_opaque = hit.is_opaque_directory()?;
                    dir_hits.push(hit);
                    if is_opaque {
                        // An opaque directory found at this layer is the last
                        // entry of the merge: deeper lower directories are
                        // hidden (Linux `ovl_lookup_single`: `val == 'y'` ->
                        // `d->stop = true`; namei.c:324-331).
                        break;
                    }
                }
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            }
        }

        if dir_hits.is_empty() {
            return Ok(LayerLookup::Negative(NegativeBinding::Absent));
        }

        let kind = if dir_hits.len() > 1 {
            PositiveKind::Merged
        } else {
            PositiveKind::Single
        };
        let upper = if dir_hits[0].layer_index() == 0 {
            Some(dir_hits.remove(0))
        } else {
            None
        };
        Ok(LayerLookup::Positive(OverlayObjectFacts {
            kind,
            upper,
            lowers: dir_hits,
        }))
    }
}
