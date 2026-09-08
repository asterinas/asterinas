// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

//! Dev/ino identity translation of the overlay namespace.
//!
//! This module owns the immutable per-mount [`IdentityPolicy`], the published
//! [`ObjectId`], and the durable lower-source identity record
//! ([`LowerIdOrigin`]).
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
//! constant `st_ino` across copy-up; the record's device/root pair is
//! resolved to a per-mount `fsid` against the mount's immutable
//! lower-layer device table.

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;

use super::{
    OverlayInode,
    copyup::workdir::WorkdirTemp,
    xattr::{OverlayRecordName, overlay_record_name},
};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::{OverlayFs, policy::XinoMode},
            layer::{Layer, LayerStack, RealObjectStack},
            real::RealObject,
        },
        vfs::{inode::Inode, xattr::XattrSetFlags},
    },
    prelude::*,
};

const U64_BITS: u32 = u64::BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectId {
    pub(super) dev: DeviceId,
    pub(super) ino: u64,
}

#[derive(Clone, Copy, Debug)]
pub(in overlayfs) struct LowerLayerIdentity {
    fsid: u64,
    container_dev_id: DeviceId,
    lower_layer_root_ino: u64,
}

pub(in overlayfs) fn collect_layer_devs(
    layer_stack: &LayerStack,
) -> (Vec<LowerLayerIdentity>, Option<usize>) {
    let layer_capacity = layer_stack.lowers.len() + usize::from(layer_stack.upper.is_some());
    let mut layer_devs: Vec<LowerLayerIdentity> = Vec::with_capacity(layer_capacity);
    let upper_layer_dev_index = if let Some(upper) = layer_stack.upper.as_ref() {
        let index = layer_devs.len();
        layer_devs.push(LowerLayerIdentity {
            fsid: upper.fsid,
            container_dev_id: upper.container_dev_id,
            lower_layer_root_ino: upper.root_dentry().inode().ino(),
        });
        Some(index)
    } else {
        None
    };
    for lower in &layer_stack.lowers {
        layer_devs.push(LowerLayerIdentity {
            fsid: lower.fsid,
            container_dev_id: lower.container_dev_id,
            lower_layer_root_ino: lower.root_dentry().inode().ino(),
        });
    }
    (layer_devs, upper_layer_dev_index)
}

#[derive(Debug)]
pub(in overlayfs) struct IdentityPolicy {
    xino_mode: XinoMode,
    overlay_dev_id: DeviceId,
    xino_shift: u32,
    is_all_layers_same_fs: bool,
    lower_layer_devs: Box<[LowerLayerIdentity]>,
    fallback_ino_allocator: AtomicU64,
}

impl IdentityPolicy {
    pub(in overlayfs) const XINO_SHIFT: u32 = 16;

    /// Upper exclusion is by position, not by value: a lower sharing the
    /// upper's underlying filesystem must keep its entry.
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
        if xino_mode == XinoMode::On && is_all_layers_same_fs {
            info!(
                "option `xino=on` is useless with all layers on the same filesystem; ignoring it"
            );
        }
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

    pub(super) fn is_xino_effective(&self) -> bool {
        if self.is_all_layers_same_fs {
            return false;
        }
        matches!(self.xino_mode, XinoMode::Auto | XinoMode::On)
    }

    pub(super) fn project_object_id(
        &self,
        layer: &Layer,
        real: &RealObject,
        is_directory: bool,
    ) -> ObjectId {
        self.project(
            layer.fsid,
            real.real_inode().ino(),
            layer.container_dev_id,
            is_directory,
        )
    }

    pub(super) fn project_object_id_from_lower_id(
        &self,
        lower_id: &LowerIdOrigin,
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

    /// The fit test rejects truncation that would alias two layers;
    /// `xino_shift == 0` must not shift by the full width.
    fn xino_fits(&self, layer_id: u64, real_ino: u64) -> bool {
        let payload_bits = U64_BITS - self.xino_shift;
        payload_bits == U64_BITS
            || (real_ino >> payload_bits == 0 && layer_id >> self.xino_shift == 0)
    }

    /// Starts at 1: ino 0 is never a valid published inode number.
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

    pub(super) fn resolve_layer_id_for_record(
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LowerIdOrigin {
    container_dev_id: DeviceId,
    lower_layer_root_ino: u64,
    real_ino: u64,
}

const ORIGIN_WIRE_VERSION: u8 = 3;

const ORIGIN_WIRE_MAGIC: u32 = 0x0000_00fb;

const ORIGIN_WIRE_HEADER_LEN: usize = 8;

const ORIGIN_WIRE_PAYLOAD_LEN: usize = 24;

const ORIGIN_WIRE_TOTAL_LEN: usize = ORIGIN_WIRE_HEADER_LEN + ORIGIN_WIRE_PAYLOAD_LEN;

const ORIGIN_WIRE_FLAGS_KNOWN: u8 = 0;

impl LowerIdOrigin {
    /// The per-mount `fsid` is deliberately not persisted: it is mount-local,
    /// so only the durable device/root pair is stored.
    fn try_from_lower(
        layer: &Layer,
        lower: &RealObject,
        lower_layer_root_ino: u64,
    ) -> Result<Self> {
        Ok(Self {
            container_dev_id: layer.container_dev_id,
            lower_layer_root_ino,
            real_ino: lower.real_inode().ino(),
        })
    }

    fn serialize(&self) -> Vec<u8> {
        let mut wire = Vec::with_capacity(ORIGIN_WIRE_TOTAL_LEN);
        wire.extend_from_slice(&ORIGIN_WIRE_MAGIC.to_ne_bytes());
        wire.push(ORIGIN_WIRE_VERSION);
        wire.push(ORIGIN_WIRE_FLAGS_KNOWN);
        wire.push(0);
        wire.push(0); // reserved header byte
        wire.extend_from_slice(&self.container_dev_id.as_encoded_u64().to_ne_bytes());
        wire.extend_from_slice(&self.lower_layer_root_ino.to_ne_bytes());
        wire.extend_from_slice(&self.real_ino.to_ne_bytes());
        wire
    }

    fn read_payload_u64(bytes: &[u8], slot: usize) -> u64 {
        let offset = ORIGIN_WIRE_HEADER_LEN + slot * 8;
        u64::from_ne_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ])
    }

    fn decode(bytes: &[u8]) -> Result<Option<Self>> {
        if bytes.len() != ORIGIN_WIRE_TOTAL_LEN {
            return Ok(None);
        }
        if u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != ORIGIN_WIRE_MAGIC {
            return Ok(None);
        }
        let version = bytes[4];
        let flags = bytes[5];
        let type_ = bytes[6];
        if version != ORIGIN_WIRE_VERSION {
            return Ok(None);
        }
        if flags & !ORIGIN_WIRE_FLAGS_KNOWN != 0 || type_ != 0 || bytes[7] != 0 {
            return Ok(None);
        }
        let Some(container_dev_id) = DeviceId::from_encoded_u64(Self::read_payload_u64(bytes, 0))
        else {
            return Ok(None);
        };
        let lower_layer_root_ino = Self::read_payload_u64(bytes, 1);
        let real_ino = Self::read_payload_u64(bytes, 2);
        Ok(Some(Self {
            container_dev_id,
            lower_layer_root_ino,
            real_ino,
        }))
    }

    pub(super) fn container_dev_id(&self) -> DeviceId {
        self.container_dev_id
    }

    pub(super) fn lower_layer_root_ino(&self) -> u64 {
        self.lower_layer_root_ino
    }

    pub(super) fn real_ino(&self) -> u64 {
        self.real_ino
    }
}

impl OverlayFs {
    /// The real-ino equality is the authenticity check: a forged or stale
    /// record must fall back to the visible-source projection.
    pub(super) fn origin_real_ino_resolves(
        &self,
        record: &LowerIdOrigin,
        facts: &RealObjectStack,
    ) -> bool {
        let Some(layer_fsid) = self
            .identity()
            .resolve_layer_id_for_record(record.container_dev_id(), record.lower_layer_root_ino())
        else {
            return false;
        };
        match facts
            .lowers
            .iter()
            .find(|lower| self.layer(lower.layer_index()).fsid == layer_fsid)
        {
            Some(retained_lower) => record.real_ino() == retained_lower.real_inode().ino(),
            None => false,
        }
    }

    pub(super) fn store_lower_id(&self, temp: &WorkdirTemp, lower: &RealObject) -> Result<()> {
        let layer = self.layer(lower.layer_index());
        let lower_layer_root_ino = self
            .layer_stack()
            .lower_layer_root_ino_for_origin(lower.layer_index())?;
        let Some(capabilities) = self.policy().upper_capabilities() else {
            return Ok(());
        };
        if !capabilities.can_store_private_xattr() {
            return Ok(());
        }
        let record = LowerIdOrigin::try_from_lower(layer, lower, lower_layer_root_ino)?;
        let value = record.serialize();
        let mut reader = VmReader::from(value.as_slice()).to_fallible();
        match OverlayInode::set_overlay_xattr(
            temp.inode(),
            temp.dentry(),
            OverlayRecordName::Origin,
            self.policy().xattr_prefix(),
            &mut reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        ) {
            Err(err) if err.error() == Errno::EOPNOTSUPP => Ok(()),
            result => result,
        }
    }

    pub(super) fn read_lower_id(&self, upper: &Arc<dyn Inode>) -> Result<Option<LowerIdOrigin>> {
        let name = overlay_record_name(OverlayRecordName::Origin, self.policy().xattr_prefix())?;
        let mut value = [0u8; ORIGIN_WIRE_TOTAL_LEN];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper.get_xattr(name, &mut writer) {
            Ok(written) => {
                let Some(record) = LowerIdOrigin::decode(&value[..written])? else {
                    return Ok(None);
                };
                let resolves = self
                    .identity()
                    .resolve_layer_id_for_record(
                        record.container_dev_id(),
                        record.lower_layer_root_ino(),
                    )
                    .is_some();
                Ok(resolves.then_some(record))
            }
            Err(err) if err.error() == Errno::ENODATA => Ok(None),
            Err(err) if err.error() == Errno::EOPNOTSUPP => Ok(None),
            // ERANGE reads as "no record": a value that overflows the fixed-length buffer
            // cannot be a canonical v3 record.
            Err(err) if err.error() == Errno::ERANGE => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[cfg(ktest)]
mod test {
    // SPDX-License-Identifier: MPL-2.0

    //! Unit tests for the xino encode/decode/fallback matrix.
    //!
    //! The assertions form a fixed case table covering passthrough, xino
    //! encoding, and the fallback paths. The tests assert the pure mapping
    //! only: no filesystem, VFS, block, or I/O fixture is constructed.

    use ostd::prelude::ktest;

    use super::*;

    fn dev(major: u16, minor: u32) -> DeviceId {
        DeviceId::new(
            device_id::MajorId::new(major),
            device_id::MinorId::new(minor),
        )
    }

    fn layer(
        fsid: u64,
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
    ) -> LowerLayerIdentity {
        LowerLayerIdentity {
            fsid,
            container_dev_id,
            lower_layer_root_ino,
        }
    }

    fn build_policy(
        overlay_dev_id: DeviceId,
        layer_devs: &[LowerLayerIdentity],
        upper_layer_dev_index: Option<usize>,
        xino_shift: u32,
        xino_mode: XinoMode,
    ) -> IdentityPolicy {
        IdentityPolicy::new(
            overlay_dev_id,
            layer_devs,
            upper_layer_dev_index,
            xino_shift,
            xino_mode,
        )
        .unwrap()
    }

    fn record(
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
        real_ino: u64,
    ) -> LowerIdOrigin {
        LowerIdOrigin {
            container_dev_id,
            lower_layer_root_ino,
            real_ino,
        }
    }

    fn valid_wire() -> Vec<u8> {
        record(dev(1, 1), 100, 0x1234).serialize()
    }

    #[ktest]
    fn policy_rejects_xino_shift_over_limit() {
        let err = IdentityPolicy::new(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100)],
            None,
            64,
            XinoMode::On,
        )
        .unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
        build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100)],
            None,
            63,
            XinoMode::On,
        );
        build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100)],
            None,
            0,
            XinoMode::On,
        );
    }

    #[ktest]
    fn same_fs_layers_pass_identity_through() {
        let policy = build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100)],
            None,
            IdentityPolicy::XINO_SHIFT,
            XinoMode::On,
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), false),
            ObjectId {
                dev: dev(1, 1),
                ino: 777
            }
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), true),
            ObjectId {
                dev: dev(1, 1),
                ino: 777
            }
        );
        assert!(!policy.is_xino_effective());
        let policy = build_policy(
            dev(9, 9),
            &[layer(7, dev(1, 1), 1), layer(0, dev(1, 1), 100)],
            Some(0),
            IdentityPolicy::XINO_SHIFT,
            XinoMode::On,
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), false),
            ObjectId {
                dev: dev(1, 1),
                ino: 777
            }
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), true),
            ObjectId {
                dev: dev(1, 1),
                ino: 777
            }
        );
        assert!(!policy.is_xino_effective());
    }

    #[ktest]
    fn xino_encodes_fsid_in_high_bits() {
        let policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::On,
        );
        let encoded = policy.project(3, 0x1234, dev(1, 1), false);
        assert_eq!(
            encoded,
            ObjectId {
                dev: dev(9, 9),
                ino: (3 << 48) | 0x1234
            }
        );
        assert_eq!(encoded.ino >> 48, 3);
        assert_eq!(encoded.ino & 0x0000_ffff_ffff_ffff, 0x1234);
        let auto_policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::Auto,
        );
        assert_eq!(auto_policy.project(3, 0x1234, dev(1, 1), false), encoded);
        assert!(auto_policy.is_xino_effective());
        let shift_63 = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            63,
            XinoMode::On,
        );
        assert_eq!(
            shift_63.project(1, 1, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: 3
            }
        );
        let shift_0 = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            0,
            XinoMode::On,
        );
        assert_eq!(
            shift_0.project(3, 0x1234, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: 0x1234
            }
        );
    }

    #[ktest]
    fn xino_off_or_overflow_takes_fallback() {
        let policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::On,
        );
        assert_eq!(
            policy.project(3, 1 << 48, dev(1, 1), false),
            ObjectId {
                dev: dev(1, 1),
                ino: 1 << 48
            }
        );
        assert_eq!(
            policy.project(3, 1 << 48, dev(1, 1), true),
            ObjectId {
                dev: dev(9, 9),
                ino: 1
            }
        );
        assert_eq!(
            policy.project(1 << 16, 5, dev(1, 1), false),
            ObjectId {
                dev: dev(1, 1),
                ino: 5
            }
        );
        let off_policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::Off,
        );
        assert_eq!(
            off_policy.project(3, 7, dev(1, 1), false),
            ObjectId {
                dev: dev(1, 1),
                ino: 7
            }
        );
        assert_eq!(
            off_policy.project(3, 7, dev(1, 1), true),
            ObjectId {
                dev: dev(9, 9),
                ino: 1
            }
        );
    }

    #[ktest]
    fn fallback_ino_allocates_from_one() {
        let policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::Off,
        );
        let first = policy.project(3, 7, dev(1, 1), true);
        let second = policy.project(3, 7, dev(1, 1), true);
        let third = policy.project(3, 7, dev(1, 1), true);
        assert_eq!(
            first,
            ObjectId {
                dev: dev(9, 9),
                ino: 1
            }
        );
        assert_eq!(
            second,
            ObjectId {
                dev: dev(9, 9),
                ino: 2
            }
        );
        assert_eq!(
            third,
            ObjectId {
                dev: dev(9, 9),
                ino: 3
            }
        );
    }

    #[ktest]
    fn resolve_layer_id_requires_unique_match() {
        let policy = build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100), layer(1, dev(2, 2), 200)],
            None,
            IdentityPolicy::XINO_SHIFT,
            XinoMode::Off,
        );
        assert_eq!(policy.resolve_layer_id_for_record(dev(1, 1), 100), Some(0));
        assert_eq!(policy.resolve_layer_id_for_record(dev(2, 2), 200), Some(1));
        assert_eq!(policy.resolve_layer_id_for_record(dev(3, 3), 300), None);
        let ambiguous = build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100), layer(1, dev(1, 1), 100)],
            None,
            IdentityPolicy::XINO_SHIFT,
            XinoMode::Off,
        );
        assert_eq!(ambiguous.resolve_layer_id_for_record(dev(1, 1), 100), None);
        let duplicate = build_policy(
            dev(9, 9),
            &[layer(0, dev(1, 1), 100), layer(0, dev(1, 1), 100)],
            None,
            IdentityPolicy::XINO_SHIFT,
            XinoMode::Off,
        );
        assert_eq!(
            duplicate.resolve_layer_id_for_record(dev(1, 1), 100),
            Some(0)
        );
        let with_upper = build_policy(
            dev(9, 9),
            &[layer(7, dev(4, 4), 1), layer(0, dev(4, 4), 2)],
            Some(0),
            IdentityPolicy::XINO_SHIFT,
            XinoMode::Off,
        );
        assert_eq!(with_upper.resolve_layer_id_for_record(dev(4, 4), 1), None);
        assert_eq!(
            with_upper.resolve_layer_id_for_record(dev(4, 4), 2),
            Some(0)
        );
    }

    #[ktest]
    fn lower_id_wire_roundtrip_preserves_identity() {
        let wire = valid_wire();
        assert_eq!(wire.len(), 32);
        let decoded = LowerIdOrigin::decode(&wire).unwrap().unwrap();
        assert_eq!(decoded.container_dev_id(), dev(1, 1));
        assert_eq!(decoded.lower_layer_root_ino(), 100);
        assert_eq!(decoded.real_ino(), 0x1234);
        let policy = build_policy(
            dev(9, 9),
            &[layer(3, dev(1, 1), 100), layer(4, dev(2, 2), 200)],
            None,
            16,
            XinoMode::On,
        );
        let via_record = policy
            .project_object_id_from_lower_id(&record(dev(1, 1), 100, 0x1234), false)
            .unwrap();
        let via_project = policy.project(3, 0x1234, dev(1, 1), false);
        assert_eq!(via_record, via_project);
        assert_eq!(
            via_record,
            ObjectId {
                dev: dev(9, 9),
                ino: (3 << 48) | 0x1234
            }
        );
        // Non-encodable directories allocate DIFFERENT fallback inos on the two
        // sides; equality is not asserted because directory stability comes from
        // the precomputed per-inode `ObjectId`.
        let dir_via_record = policy
            .project_object_id_from_lower_id(&record(dev(1, 1), 100, 1 << 48), true)
            .unwrap();
        let dir_via_project = policy.project(3, 1 << 48, dev(1, 1), true);
        assert_eq!(dir_via_record.dev, dev(9, 9));
        assert_eq!(dir_via_project.dev, dev(9, 9));
        assert_ne!(dir_via_record.ino, dir_via_project.ino);
        assert_eq!(
            policy.project_object_id_from_lower_id(&record(dev(9, 9), 999, 5), false),
            None
        );
    }

    #[ktest]
    fn lower_id_wire_decode_rejects_malformed() {
        let mut short = valid_wire();
        short.truncate(31);
        assert_eq!(LowerIdOrigin::decode(&short).unwrap(), None);
        let mut long = valid_wire();
        long.push(0);
        assert_eq!(LowerIdOrigin::decode(&long).unwrap(), None);
        let mut magic = valid_wire();
        magic[0] ^= 0xff;
        assert_eq!(LowerIdOrigin::decode(&magic).unwrap(), None);
        for version in [0u8, 2, 4] {
            let mut wire = valid_wire();
            wire[4] = version;
            assert_eq!(LowerIdOrigin::decode(&wire).unwrap(), None);
        }
        let mut flags = valid_wire();
        flags[5] = 0x01;
        assert_eq!(LowerIdOrigin::decode(&flags).unwrap(), None);
        let mut type_byte = valid_wire();
        type_byte[6] = 1;
        assert_eq!(LowerIdOrigin::decode(&type_byte).unwrap(), None);
        let mut reserved = valid_wire();
        reserved[7] = 1;
        assert_eq!(LowerIdOrigin::decode(&reserved).unwrap(), None);
        let mut invalid_dev = valid_wire();
        invalid_dev[8..16]
            .copy_from_slice(&device_id::encode_device_numbers(0x1000, 0).to_ne_bytes());
        assert_eq!(LowerIdOrigin::decode(&invalid_dev).unwrap(), None);
    }
}
