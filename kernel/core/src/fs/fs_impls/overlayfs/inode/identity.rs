// SPDX-License-Identifier: MPL-2.0

//! Dev/ino identity translation of the overlay namespace.
//!
//! This module owns the immutable per-mount [`IdentityPolicy`], the published
//! [`ObjectId`], and the durable lower-source identity record
//! ([`LowerIdOrigin`]).
//!
//! The **xino matrix** decides, for one layer and one real inode, which
//! `st_dev`/`st_ino` pair the overlay publishes. Its two inputs are resolved
//! once at mount from the mount option and the layer set:
//!
//! - **same-fs passthrough** (`is_same_fs_passthrough`) — every layer shares
//!   one underlying filesystem, so `st_ino` matches the underlying inode and
//!   `st_dev` is uniform;
//! - **xino effective** (`is_xino_effective`) — the overlay publishes its own
//!   `st_dev` and an encoded `st_ino` (layer `fsid` in the high `xino_shift`
//!   bits, real ino in the payload); `xino=on` forces this even on a
//!   same-filesystem mount;
//! - **xino off** — directories report the overlay `st_dev` plus a
//!   saturating allocated ino; non-directories report the underlying
//!   dev/ino; an ino that does not fit the xino payload falls back per object
//!   to the xino-off behavior (explicit, never silently wrong).
//!
//! A **lower-id record** is the durable ([`LayerIdentity`], `real_ino`)
//! provenance that copy-up persists on the upper inode.
//! [`OverlayFs::project_origin_object_id`] reads such a record, resolves its
//! durable pair to the retained mount-local layer, and feeds it back through
//! the same xino matrix so the object keeps a constant `st_ino` across
//! copy-up.
//!
//! # Record format
//!
//! The record is a custom durable format: an 8-byte header (magic
//! `0x0000_00fb`, version `3`, flags, reserved) plus a 24-byte payload
//! (encoded container device id, layer root inode number, real inode number),
//! 32 bytes in total. The format is private to this implementation, so no
//! other overlay implementation can decode it and no cross-implementation
//! origin resolution is possible. Consequently `uuid` has local semantics
//! only: the record carries no UUID, so no UUID is encoded in or restored
//! from it.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;

use super::{OverlayInode, copyup::workdir::WorkdirTemp, xattr::OverlayRecordName};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::{OverlayFs, policy::XinoMode},
            layer::{Layer, LayerIdentity},
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

#[derive(Debug)]
pub(in overlayfs) struct IdentityPolicy {
    overlay_dev_id: DeviceId,
    xino_shift: u32,
    is_same_fs_passthrough: bool,
    is_xino_effective: bool,
    fallback_ino_allocator: AtomicU64,
}

impl IdentityPolicy {
    pub(in overlayfs) const XINO_SHIFT: u32 = 16;

    /// Resolves the mount option and layer-set predicate into the two flags once, at mount time.
    pub(in overlayfs) fn new(
        overlay_dev_id: DeviceId,
        xino_shift: u32,
        xino_mode: XinoMode,
        is_all_layers_same_fs: bool,
    ) -> Result<Self> {
        if xino_shift > 63 {
            return_errno_with_message!(Errno::EINVAL, "invalid overlay xino shift");
        }
        let is_same_fs_passthrough = is_all_layers_same_fs && xino_mode != XinoMode::On;
        let is_xino_effective = match xino_mode {
            XinoMode::Off => false,
            XinoMode::Auto => !is_all_layers_same_fs,
            XinoMode::On => true,
        };
        Ok(Self {
            overlay_dev_id,
            xino_shift,
            is_same_fs_passthrough,
            is_xino_effective,
            fallback_ino_allocator: AtomicU64::new(0),
        })
    }

    pub(super) fn project(
        &self,
        layer_id: u64,
        real_ino: u64,
        origin_dev: DeviceId,
        is_directory: bool,
    ) -> ObjectId {
        if self.is_same_fs_passthrough {
            return ObjectId {
                dev: origin_dev,
                ino: real_ino,
            };
        }
        if self.is_xino_effective && self.xino_fits(layer_id, real_ino) {
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

    /// The fit test rejects truncation that would alias two layers; `xino_shift == 0` is handled.
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
}

/// The durable provenance of a copied-up object: its source layer identity and real inode number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LowerIdOrigin {
    identity: LayerIdentity,
    real_ino: u64,
}

const ORIGIN_WIRE_VERSION: u8 = 3;

const ORIGIN_WIRE_MAGIC: u32 = 0x0000_00fb;

const ORIGIN_WIRE_HEADER_LEN: usize = 8;

const ORIGIN_WIRE_PAYLOAD_LEN: usize = 24;

const ORIGIN_WIRE_TOTAL_LEN: usize = ORIGIN_WIRE_HEADER_LEN + ORIGIN_WIRE_PAYLOAD_LEN;

const ORIGIN_WIRE_FLAGS_KNOWN: u8 = 0;

const ORIGIN_WIRE_HEADER: [u8; ORIGIN_WIRE_HEADER_LEN] = {
    let magic = ORIGIN_WIRE_MAGIC.to_ne_bytes();
    [
        magic[0],
        magic[1],
        magic[2],
        magic[3],
        ORIGIN_WIRE_VERSION,
        ORIGIN_WIRE_FLAGS_KNOWN,
        0,
        0, // reserved header byte
    ]
};

impl LowerIdOrigin {
    fn serialize(&self) -> Vec<u8> {
        let mut wire = Vec::with_capacity(ORIGIN_WIRE_TOTAL_LEN);
        wire.extend_from_slice(&ORIGIN_WIRE_HEADER);
        wire.extend_from_slice(
            &self
                .identity
                .container_dev_id
                .as_encoded_u64()
                .to_ne_bytes(),
        );
        wire.extend_from_slice(&self.identity.root_ino.to_ne_bytes());
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
        let root_ino = Self::read_payload_u64(bytes, 1);
        let real_ino = Self::read_payload_u64(bytes, 2);
        Ok(Some(Self {
            identity: LayerIdentity {
                container_dev_id,
                root_ino,
            },
            real_ino,
        }))
    }

    pub(super) fn real_ino(&self) -> u64 {
        self.real_ino
    }
}

impl OverlayFs {
    /// Resolves a durable record to the retained lower layer and authenticates its inode.
    pub(super) fn resolve_retained_origin_layer(
        &self,
        origin: &LowerIdOrigin,
        lowers: &[RealObject],
    ) -> Option<&Layer> {
        let origin_layer = self.layer_stack().resolve_lower_layer(&origin.identity)?;
        let retained = lowers
            .iter()
            .find(|lower| core::ptr::eq(origin_layer, self.layer(lower.layer_index())))?;
        if origin.real_ino() == retained.real_inode().ino() {
            Some(origin_layer)
        } else {
            None
        }
    }

    /// Derives the retained origin of an upper object from one record read; failures fall back.
    pub(super) fn project_origin_object_id(
        &self,
        upper: &Arc<dyn Inode>,
        lowers: &[RealObject],
        is_directory: bool,
    ) -> Result<Option<ObjectId>> {
        let Some(origin) = self.read_lower_id(upper)? else {
            return Ok(None);
        };
        let Some(layer) = self.resolve_retained_origin_layer(&origin, lowers) else {
            return Ok(None);
        };
        Ok(Some(self.identity().project(
            layer.fsid,
            origin.real_ino(),
            layer.container_dev_id,
            is_directory,
        )))
    }

    pub(super) fn store_lower_id(&self, temp: &WorkdirTemp, lower: &RealObject) -> Result<()> {
        // The capability is read before the record is built: a private xattr persists it.
        if !self.policy().can_store_private_xattr() {
            // Origin records are best-effort: skip them when private xattrs are unavailable.
            return Ok(());
        }
        let record = LowerIdOrigin {
            identity: self.layer(lower.layer_index()).identity(),
            real_ino: lower.real_inode().ino(),
        };
        let value = record.serialize();
        let mut reader = VmReader::from(value.as_slice()).to_fallible();
        match OverlayInode::set_overlay_xattr(
            temp.inode(),
            temp.dentry(),
            OverlayRecordName::Origin,
            self.policy().xattr_namespace(),
            &mut reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        ) {
            // Origin records are best-effort: an unsupported xattr must not abort copy-up.
            Err(err) if matches!(err.error(), Errno::EOPNOTSUPP | Errno::EPERM) => Ok(()),
            result => result,
        }
    }

    pub(super) fn read_lower_id(&self, upper: &Arc<dyn Inode>) -> Result<Option<LowerIdOrigin>> {
        let name =
            OverlayRecordName::Origin.construct_xattr_name(self.policy().xattr_namespace())?;
        let mut value = [0u8; ORIGIN_WIRE_TOTAL_LEN];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper.get_xattr(name, &mut writer) {
            Ok(written) => LowerIdOrigin::decode(&value[..written]),
            Err(err) if err.error() == Errno::ENODATA => Ok(None),
            Err(err) if err.error() == Errno::EOPNOTSUPP => Ok(None),
            // `ERANGE` reads as "no record": an oversized value cannot be canonical.
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

    fn build_policy(
        overlay_dev_id: DeviceId,
        xino_shift: u32,
        xino_mode: XinoMode,
        is_all_layers_same_fs: bool,
    ) -> IdentityPolicy {
        IdentityPolicy::new(overlay_dev_id, xino_shift, xino_mode, is_all_layers_same_fs).unwrap()
    }

    fn record(
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
        real_ino: u64,
    ) -> LowerIdOrigin {
        LowerIdOrigin {
            identity: LayerIdentity {
                container_dev_id,
                root_ino: lower_layer_root_ino,
            },
            real_ino,
        }
    }

    fn valid_wire() -> Vec<u8> {
        record(dev(1, 1), 100, 0x1234).serialize()
    }

    #[ktest]
    fn policy_rejects_xino_shift_over_limit() {
        let err = IdentityPolicy::new(dev(9, 9), 64, XinoMode::On, false).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
        build_policy(dev(9, 9), 63, XinoMode::On, false);
        build_policy(dev(9, 9), 0, XinoMode::On, false);
    }

    #[ktest]
    fn xino_on_forces_encoding_on_same_fs() {
        let policy = build_policy(dev(9, 9), IdentityPolicy::XINO_SHIFT, XinoMode::On, true);
        assert_eq!(
            policy.project(5, 777, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: (5 << 48) | 777
            }
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), true),
            ObjectId {
                dev: dev(9, 9),
                ino: (5 << 48) | 777
            }
        );
        assert!(policy.is_xino_effective);
        // The same force applies when an upper shares the lowers' filesystem.
        let policy = build_policy(dev(9, 9), IdentityPolicy::XINO_SHIFT, XinoMode::On, true);
        assert_eq!(
            policy.project(5, 777, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: (5 << 48) | 777
            }
        );
        assert_eq!(
            policy.project(5, 777, dev(1, 1), true),
            ObjectId {
                dev: dev(9, 9),
                ino: (5 << 48) | 777
            }
        );
        assert!(policy.is_xino_effective);
    }

    #[ktest]
    fn xino_encodes_fsid_in_high_bits() {
        let policy = build_policy(dev(9, 9), 16, XinoMode::On, false);
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
        let auto_policy = build_policy(dev(9, 9), 16, XinoMode::Auto, false);
        assert_eq!(auto_policy.project(3, 0x1234, dev(1, 1), false), encoded);
        assert!(auto_policy.is_xino_effective);
        let shift_63 = build_policy(dev(9, 9), 63, XinoMode::On, false);
        assert_eq!(
            shift_63.project(1, 1, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: 3
            }
        );
        let shift_0 = build_policy(dev(9, 9), 0, XinoMode::On, false);
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
        let policy = build_policy(dev(9, 9), 16, XinoMode::On, false);
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
        let off_policy = build_policy(dev(9, 9), 16, XinoMode::Off, false);
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
        let policy = build_policy(dev(9, 9), 16, XinoMode::Off, false);
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
    fn lower_id_wire_roundtrip_preserves_identity() {
        let wire = valid_wire();
        assert_eq!(wire.len(), 32);
        let decoded = LowerIdOrigin::decode(&wire).unwrap().unwrap();
        assert_eq!(decoded.identity.container_dev_id, dev(1, 1));
        assert_eq!(decoded.identity.root_ino, 100);
        assert_eq!(decoded.real_ino(), 0x1234);
        let policy = build_policy(dev(9, 9), 16, XinoMode::On, false);
        assert_eq!(
            policy.project(3, 0x1234, dev(1, 1), false),
            ObjectId {
                dev: dev(9, 9),
                ino: (3 << 48) | 0x1234
            }
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
