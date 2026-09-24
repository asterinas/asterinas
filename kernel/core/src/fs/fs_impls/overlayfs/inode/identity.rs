// SPDX-License-Identifier: MPL-2.0

//! Dev/ino identity: the pair an overlay object publishes, and how a mount projects it.
//!
//! # Overlay identity projection
//!
//! Overlayfs exposes files from different underlying filesystems in one visible filesystem. Within
//! one filesystem an inode should keep its identity (the `st_dev`/`st_ino` pair that a `stat` or the
//! like reports) unique and stable, that is:
//! - the identity of one inode does not change within one mount;
//! - no two inodes share one inode number; and
//! - the identity of one inode ideally also survives a remount, because some overlayfs features
//!   durably record an inode's identity.
//!
//! Filesystems, however, do not coordinate identities among themselves, so passing an underlying
//! identity through as it is can break the properties above. Overlayfs therefore keeps an identity
//! system of its own: it maps the real identities taken from the different underlying filesystems
//! onto one identity space by an encoding of its own, which is called projection.
//!
//! Overlayfs has two ways to project an identity, resolved once at mount time into the mount's
//! [`IdentityPolicy`]:
//! - `SameFs` — every layer really lives on one underlying filesystem, so that filesystem already
//!   provides the identity guarantees and overlayfs may pass its identities through.
//!   - To decide whether the layers share one filesystem, the mount checks the filesystem instance
//!   each real layer belongs to and assigns one `fsid` per instance, topmost layer first; when all
//!   layers belong to one filesystem, only `fsid = 0` is assigned.
//! - `Xino` — overlayfs provides the identity guarantees itself, in particular when the layers live
//!   on several filesystem instances (a single-instance mount can force it too).
//!   - Overlayfs takes an anonymous device id of its own, distinct from every existing
//!     filesystem, so that device numbers cannot collide.
//!   - Each overlayfs inode is given an inode number assembled from the `fsid` of the real filesystem
//!     the inode lives on and the inode number it has there, `real_ino`:
//! ```text
//!  63         x   x-1                   0
//!  +----------+---+---------------------+
//!  |   fsid   |0/1|      real_ino       |
//!  +----------+---+---------------------+
//! ```
//! - - The width of the `fsid` field follows the largest `fsid` the mount assigned.
//! - - The `0/1` bit marks whether the number is a fallback number (see below).
//!
//! `Xino` is the default (`xino=auto`); overlayfs falls back in the following cases:
//! - Per object: while the xino is in use, a real inode whose `real_ino` may be too long for the
//!   field. Overlayfs then falls back on its own: The `0/1` bit is set on at fallback, so it cannot
//!   collide with an encoded one.
//! - Mount-wide, in either of two cases:
//!   - the mount was asked for `xino=off`; or
//!   - the mount keeps `xino=auto` and can neither stay read-only nor store a private record on its
//!     upper. A read-only mount never copies up, so an inode's real identity cannot change under it;
//!     a mount whose upper can store private xattrs keeps the origin record across a copy-up, so
//!     the identity stays stable there too. Either case can hold an encoded number for an object's
//!     whole life, which is what xino needs. When neither holds, the whole mount falls back.
//!
//! In `Fallback` mode, every directory allocates a unique inode number from the mount, and uses the
//! mount's anonymous device id as its device id. The allocation is done by a monotonic `u64`
//! allocator. A non-directory keeps its real identity (both device and inode number) as it is.
//!
//! A directory's number is allocated by this mount alone, so a directory's identity does not survive
//! a remount: another mount of the same layers has its own numbers. A non-directory keeps its
//! layer's own pair, so a non-directory's identity does survive a remount. A directory's pair and a
//! non-directory's cannot collide, because they uses different devices. Two non-directories can
//! share a pair when two of the layers live on filesystems that report one device number. That means
//! two instances of one device, which is rarely seen.
//!
//! # Public types
//!
//! - [`IdentityPolicy`]: the mount's resolved way of projecting identities.
//! - [`ObjectRealId`]: `(fsid, real_ino)`, which can be used to build a visible id.
//! - [`ObjectVisibleId`]: `(st_dev, st_ino)`. Because fallback allocates from a monotonic allocator, a
//!   visible id cannot in general be decoded back into a real id. Once set for an inode, it will not
//!   change during the mount lifecycle.
//! - [`ObjectOriginRecord`]: `(magic, container_dev_id, root_ino, real_ino)`, which durably records
//!   the original lower inode of an upper inode that a copy-up produced, so that the identity
//!   survives across mounts.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;
use hashbrown::HashMap;

use super::{copyup::workdir::WorkdirTemp, xattr::OverlayXattrType};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::{
                OverlayFs,
                mount::{XinoMode, policy::MountPolicy},
            },
            layer::{Layer, LayerStack},
            real::{RealObject, RealObjectStack},
        },
        pseudofs::AnonDeviceId,
        vfs::{file_system::FileSystem, path::Dentry},
    },
    prelude::*,
};

/// The mount's resolved way of projecting identities.
#[derive(Debug)]
pub(in overlayfs) struct IdentityPolicy {
    /// The form this mount resolved to.
    mode: IdentityMode,
    /// The device the mount publishes for the numbers it allocates itself.
    overlay_device: AnonDeviceId,
    /// The numbers this mount allocates; absent when the resolved form allocates none.
    fallback_ino: Option<AtomicU64>,
    upper_numbers: Option<LayerNumbers>,
    /// The numbers of every lower, topmost first — the layers an origin record can name.
    lower_numbers: Vec<LayerNumbers>,
}

/// How this mount projects the dev/ino pair an object publishes.
#[derive(Debug)]
enum IdentityMode {
    /// Every layer is on one filesystem, where a layer's own numbers are already unique.
    SameFs,
    /// A directory takes a number this mount allocates; anything else keeps its layer's numbers.
    Fallback,
    /// The overlay's own device publishes one number per object, unique across every layer.
    ///
    /// The layer's `fsid` occupies the high bits of that number.
    Xino { xino_shift: u32 },
}

/// The real id of one real object inside this mount.
///
/// It stays the same as long as that layer numbers its inodes stably, and it is the key the
/// identity-reuse cache files an object under.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ObjectRealId {
    fsid: u64,
    real_ino: u64,
}

impl ObjectRealId {
    /// Encodes this id into one published inode number, or `None` when its real inode number does
    /// not fit the payload.
    ///
    /// The layer's `fsid` sits above the lowest bit, which the mount keeps clear.
    fn encode_xino(&self, xino_shift: u32) -> Option<u64> {
        if self.real_ino >> xino_shift != 0 {
            return None;
        }
        let fsid_shift = xino_shift + 1;
        Some((self.fsid << fsid_shift) | self.real_ino)
    }
}

/// The published identity of one overlay object.
///
/// The mount projects it once for the object's life, and the ids behind it cannot be read back out
/// of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectVisibleId {
    pub(super) dev: DeviceId,
    pub(super) ino: u64,
}

/// The numbers the id system keeps for one layer — what a durable layer name resolves to.
#[derive(Clone, Copy, Debug)]
struct LayerNumbers {
    fsid: u64,
    container_dev_id: DeviceId,
    root_ino: u64,
}

/// The durable form of a real id.
///
/// It names its layer the way any mount can resolve it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ObjectOriginRecord {
    /// The word that marks these bytes as an origin record.
    magic: u64,
    /// The layer's backing device, in the encoded form of a device number.
    container_dev_id: u64,
    /// The inode number of the layer's pinned root.
    root_ino: u64,
    /// The inode number of the object in that layer.
    real_ino: u64,
}

impl ObjectOriginRecord {
    /// The word an origin record starts with.
    const MAGIC: u64 = 0x0000_00fb;

    /// Reads one record back, or `None` when the bytes are not a record this mount wrote.
    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != size_of::<Self>() {
            return None;
        }
        let record = Self::from_bytes(bytes);
        if record.magic != Self::MAGIC
            || DeviceId::from_encoded_u64(record.container_dev_id).is_none()
        {
            return None;
        }
        Some(record)
    }
}

impl IdentityPolicy {
    /// Assembles the way of projecting identities from the facts a mount resolved.
    ///
    /// The `xino=auto` degradation follows Linux `ovl_lower_dir` and `ovl_make_workdir`, which turn
    /// `xino=auto` into `xino=off` when the mount can keep no persistent identity for an object that
    /// a copy-up replaces.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/super.c#L437-L446>,
    /// <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/super.c#L745-L755>.
    fn assemble(
        overlay_device: AnonDeviceId,
        xino_mode: XinoMode,
        fs_count: u64,
        is_effective_read_only: bool,
        can_store_private_xattr: bool,
        upper_numbers: Option<LayerNumbers>,
        lower_numbers: Vec<LayerNumbers>,
    ) -> Self {
        let is_all_layers_same_fs = fs_count == 1;
        let mode = if is_all_layers_same_fs && xino_mode != XinoMode::On {
            IdentityMode::SameFs
        } else if xino_mode == XinoMode::Off
            || (xino_mode == XinoMode::Auto && !(is_effective_read_only || can_store_private_xattr))
        {
            IdentityMode::Fallback
        } else {
            IdentityMode::Xino {
                // The payload width follows the number of filesystems the mount has, as in Linux
                // `ovl_get_layers`.
                //
                // Reference:
                // <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/super.c#L1159>.
                xino_shift: 64 - (fs_count.max(2) - 1).ilog2() - 2,
            }
        };
        let fallback_ino = match mode {
            IdentityMode::SameFs => None,
            IdentityMode::Fallback | IdentityMode::Xino { .. } => Some(AtomicU64::new(1)),
        };
        Self {
            mode,
            overlay_device,
            fallback_ino,
            upper_numbers,
            lower_numbers,
        }
    }

    /// Projects the pair one object publishes; the object keeps it for life.
    pub(super) fn project(&self, real_id: ObjectRealId, is_directory: bool) -> ObjectVisibleId {
        let layer_pair = ObjectVisibleId {
            dev: self.device_of_fsid(real_id.fsid),
            ino: real_id.real_ino,
        };
        match self.mode {
            IdentityMode::SameFs => layer_pair,
            IdentityMode::Fallback => {
                if is_directory {
                    ObjectVisibleId {
                        dev: self.overlay_device.id(),
                        ino: self.allocate_fallback_ino(),
                    }
                } else {
                    layer_pair
                }
            }
            IdentityMode::Xino { xino_shift } => match real_id.encode_xino(xino_shift) {
                Some(ino) => ObjectVisibleId {
                    dev: self.overlay_device.id(),
                    ino,
                },
                None if is_directory => ObjectVisibleId {
                    dev: self.overlay_device.id(),
                    ino: self.xino_fallback_ino(xino_shift),
                },
                None => layer_pair,
            },
        }
    }

    /// Returns the real id of one real object inside this mount.
    pub(super) fn real_id_of(&self, real: &RealObject) -> ObjectRealId {
        ObjectRealId {
            fsid: self.layer(real.layer_index()).fsid,
            real_ino: real.real_inode().ino(),
        }
    }

    /// Builds the durable record of one object, naming its layer the way any mount can read.
    fn origin_record_of(&self, source: &RealObject) -> ObjectOriginRecord {
        let layer = self.layer(source.layer_index());
        ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: layer.container_dev_id.as_encoded_u64(),
            root_ino: layer.root_ino,
            real_ino: source.real_inode().ino(),
        }
    }

    /// Whether the published pairs are the layers' own numbers.
    pub(super) fn is_same_fs_passthrough(&self) -> bool {
        matches!(self.mode, IdentityMode::SameFs)
    }

    /// Returns the layer `layer_index` selects.
    ///
    /// Index 0 is the upper; index `i` is the `i`-th lower, topmost first.
    fn layer(&self, layer_index: usize) -> &LayerNumbers {
        match layer_index {
            0 => self
                .upper_numbers
                .as_ref()
                .expect("a real object with layer index 0 references the upper layer"),
            _ => self
                .lower_numbers
                .get(layer_index - 1)
                .expect("a real object references a configured lower layer"),
        }
    }

    /// Returns the device the layers of one `fsid` are on.
    fn device_of_fsid(&self, fsid: u64) -> DeviceId {
        self.upper_numbers
            .iter()
            .chain(self.lower_numbers.iter())
            .find(|layer| layer.fsid == fsid)
            .expect("a published fsid belongs to a configured layer")
            .container_dev_id
    }

    /// Returns the `fsid` of the layer an origin record points at.
    ///
    /// `None` means no single lower of this mount matches the record.
    fn fsid_of_origin(&self, origin: &ObjectOriginRecord) -> Option<u64> {
        let mut matched: Option<u64> = None;
        for layer in self.lower_numbers.iter() {
            if layer.container_dev_id.as_encoded_u64() != origin.container_dev_id
                || layer.root_ino != origin.root_ino
            {
                continue;
            }
            match matched {
                None => matched = Some(layer.fsid),
                Some(existing) if existing == layer.fsid => {}
                Some(_) => return None,
            }
        }
        matched
    }

    /// Returns the next number this mount allocates.
    ///
    /// A self-allocated number has to stay distinguishable from an encoded one, so
    /// `xino_fallback_ino` sets the lowest encoding bit on it — the bit `encode_xino` keeps clear.
    /// Linux marks a self-allocated number the same way with `ovl_map_dev_ino` and `ovl_map_ino`.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/inode.c#L120-L121>,
    /// <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/inode.c#L867-L870>.
    fn allocate_fallback_ino(&self) -> u64 {
        let range = self
            .fallback_ino
            .as_ref()
            .expect("a form that allocates numbers keeps one");
        range.fetch_add(1, Ordering::Relaxed)
    }

    /// Returns a number this mount allocates with the bit the encoding keeps clear set on it.
    fn xino_fallback_ino(&self, xino_shift: u32) -> u64 {
        let cleared_bit = 1 << xino_shift;
        let ino = self.allocate_fallback_ino();
        cleared_bit | (ino & (cleared_bit - 1))
    }
}

impl OverlayFs {
    /// Resolves one mount's way of projecting identities; `xino=auto` resolves to the encoding only
    /// on a read-only mount or one whose upper stores a private record.
    pub(in overlayfs) fn new_identity_policy(
        overlay_device: AnonDeviceId,
        layers: &LayerStack,
        xino_mode: XinoMode,
        is_effective_read_only: bool,
        policy: &MountPolicy,
    ) -> IdentityPolicy {
        let mut fses: Vec<Arc<dyn FileSystem>> = Vec::new();
        let mut fsid_by_address: HashMap<usize, u64> = HashMap::new();
        // The upper is assigned first, so its fsid is always 0 (Linux `ovl_layer[].fsid`).
        //
        // Reference:
        // <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/ovl_entry.h#L33-L42>.
        let mut assign_fsid = |layer: &Layer, fses: &mut Vec<Arc<dyn FileSystem>>| -> u64 {
            let address = Arc::as_ptr(layer.fs()) as *const () as usize;
            if let Some(fsid) = fsid_by_address.get(&address) {
                return *fsid;
            }
            let fsid = fses.len() as u64;
            fses.push(layer.fs().clone());
            fsid_by_address.insert(address, fsid);
            fsid
        };
        let upper_numbers = layers.upper_layer().ok().map(|upper| LayerNumbers {
            fsid: assign_fsid(upper, &mut fses),
            container_dev_id: upper.container_dev_id(),
            root_ino: upper.root_dentry().inode().ino(),
        });
        let lower_numbers: Vec<LayerNumbers> = layers
            .lower_layers()
            .iter()
            .map(|lower| LayerNumbers {
                fsid: assign_fsid(lower, &mut fses),
                container_dev_id: lower.container_dev_id(),
                root_ino: lower.root_dentry().inode().ino(),
            })
            .collect();
        IdentityPolicy::assemble(
            overlay_device,
            xino_mode,
            fses.len() as u64,
            is_effective_read_only,
            policy.can_store_private_xattr(),
            upper_numbers,
            lower_numbers,
        )
    }

    /// Returns the real id recorded in the upper's durable origin record, resolved onto a lower of this
    /// mount.
    ///
    /// `None` covers an absent, undecodable, or unreadable record — the unreadable one warns once — and
    /// a record that matches no single lower.
    pub(super) fn origin_of(&self, upper: &Dentry) -> Option<ObjectRealId> {
        let mut value = [0u8; size_of::<ObjectOriginRecord>()];
        let read = OverlayXattrType::Origin.get_value_from(
            upper,
            self.policy().xattr_namespace(),
            &mut value,
        );
        let record = match read {
            Ok(written) => ObjectOriginRecord::decode(&value[..written])?,
            // `ENODATA` and `EOPNOTSUPP` mean no record is there, and `ERANGE` means the stored
            // value is longer than a record: all three read as "no record".
            Err(err)
                if matches!(
                    err.error(),
                    Errno::ENODATA | Errno::EOPNOTSUPP | Errno::ERANGE
                ) =>
            {
                return None;
            }
            Err(err) => {
                warn!(
                    "failed to read the origin record; treating the object as a pure upper: {:?}",
                    err
                );
                return None;
            }
        };
        let fsid = self.identity().fsid_of_origin(&record)?;
        Some(ObjectRealId {
            fsid,
            real_ino: record.real_ino,
        })
    }

    /// Returns the real id one projected stack resolves to: the id of its visible source.
    ///
    /// Two kinds of object answer differently: a directory takes its topmost retained lower's number
    /// — the one its identity must survive a copy-up as — and a non-directory upper takes the durable
    /// record it carries.
    pub(super) fn real_id_of_stack(&self, facts: &RealObjectStack) -> ObjectRealId {
        let visible = facts.visible_source();
        let is_directory = visible.real_inode().type_().is_directory();
        let identity = self.identity();
        if facts.upper.is_none() {
            // A lower-only winner is its own identity: the visible source supplies the pair.
            identity.real_id_of(visible)
        } else if is_directory {
            // A directory never reads a record: its topmost retained lower supplies the pair.
            match facts.lowers.first() {
                Some(topmost_lower) => identity.real_id_of(topmost_lower),
                None => identity.real_id_of(visible),
            }
        } else {
            // A non-directory upper is keyed by its durable record, never by the name probe.
            self.origin_of(visible.dentry())
                .unwrap_or_else(|| identity.real_id_of(visible))
        }
    }

    /// Writes `source`'s real id onto the staged temp as the object's durable record: `Ok(true)` when
    /// it landed, `Ok(false)` when the mount cannot hold private records or the filesystem refused one.
    pub(super) fn record_origin(&self, temp: &WorkdirTemp, source: &RealObject) -> Result<bool> {
        if !self.policy().can_store_private_xattr() {
            return Ok(false);
        }
        let record = self.identity().origin_record_of(source);
        match OverlayXattrType::Origin.set_value_on(
            temp.dentry(),
            self.policy().xattr_namespace(),
            Some(record.as_bytes()),
        ) {
            Ok(()) => Ok(true),
            Err(err) if matches!(err.error(), Errno::EOPNOTSUPP | Errno::EPERM) => Ok(false),
            Err(err) => Err(err),
        }
    }
}

#[cfg(ktest)]
mod test {
    //! Unit tests for the pure mapping this module owns: the mount's form, the layers' numbers,
    //! the published pairs, and the record's wire form.

    use ostd::prelude::ktest;

    use super::*;

    fn dev(major: u16, minor: u32) -> DeviceId {
        DeviceId::new(
            device_id::MajorId::new(major),
            device_id::MinorId::new(minor),
        )
    }

    fn build_policy(
        xino_mode: XinoMode,
        fs_count: u64,
        is_effective_read_only: bool,
        can_store_private_xattr: bool,
        upper_numbers: Option<LayerNumbers>,
        lower_numbers: Vec<LayerNumbers>,
    ) -> IdentityPolicy {
        IdentityPolicy::assemble(
            AnonDeviceId::acquire().expect("the unit test takes an anonymous device id"),
            xino_mode,
            fs_count,
            is_effective_read_only,
            can_store_private_xattr,
            upper_numbers,
            lower_numbers,
        )
    }

    fn numbers(fsid: u64, container_dev_id: DeviceId, root_ino: u64) -> LayerNumbers {
        LayerNumbers {
            fsid,
            container_dev_id,
            root_ino,
        }
    }

    fn real_id(fsid: u64, real_ino: u64) -> ObjectRealId {
        ObjectRealId { fsid, real_ino }
    }

    fn record(
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
        real_ino: u64,
    ) -> ObjectOriginRecord {
        let layer = numbers(0, container_dev_id, lower_layer_root_ino);
        ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: layer.container_dev_id.as_encoded_u64(),
            root_ino: layer.root_ino,
            real_ino,
        }
    }

    fn valid_wire() -> Vec<u8> {
        record(dev(1, 1), 100, 0x1234).as_bytes().to_vec()
    }

    #[ktest]
    fn identity_mode_resolves_from_mount_facts() {
        let same_fs_off = build_policy(XinoMode::Off, 1, false, false, None, Vec::new());
        assert!(matches!(same_fs_off.mode, IdentityMode::SameFs));
        assert!(same_fs_off.fallback_ino.is_none());

        let same_fs_auto = build_policy(XinoMode::Auto, 1, false, false, None, Vec::new());
        assert!(matches!(same_fs_auto.mode, IdentityMode::SameFs));
        assert!(same_fs_auto.fallback_ino.is_none());

        let spanning_off = build_policy(XinoMode::Off, 2, false, false, None, Vec::new());
        assert!(matches!(spanning_off.mode, IdentityMode::Fallback));
        assert!(spanning_off.fallback_ino.is_some());

        let spanning_auto = build_policy(XinoMode::Auto, 2, false, false, None, Vec::new());
        assert!(matches!(spanning_auto.mode, IdentityMode::Fallback));
        assert!(spanning_auto.fallback_ino.is_some());

        let auto_read_only = build_policy(XinoMode::Auto, 2, true, false, None, Vec::new());
        assert!(matches!(auto_read_only.mode, IdentityMode::Xino { .. }));
        assert!(auto_read_only.fallback_ino.is_some());

        let auto_recording = build_policy(XinoMode::Auto, 2, false, true, None, Vec::new());
        assert!(matches!(auto_recording.mode, IdentityMode::Xino { .. }));
        assert!(auto_recording.fallback_ino.is_some());

        let on_same_fs = build_policy(XinoMode::On, 1, false, false, None, Vec::new());
        assert!(matches!(on_same_fs.mode, IdentityMode::Xino { .. }));
        assert!(on_same_fs.fallback_ino.is_some());

        let on_spanning = build_policy(XinoMode::On, 2, true, true, None, Vec::new());
        assert!(matches!(on_spanning.mode, IdentityMode::Xino { .. }));
        assert!(on_spanning.fallback_ino.is_some());
    }

    #[ktest]
    fn xino_on_forces_encoding_on_same_fs() {
        let upper = numbers(0, dev(1, 1), 100);
        let policy = build_policy(XinoMode::On, 1, false, false, Some(upper), Vec::new());
        let encoded = ObjectVisibleId {
            dev: policy.overlay_device.id(),
            ino: 777,
        };
        assert_eq!(policy.project(real_id(0, 777), false), encoded);
        assert_eq!(policy.project(real_id(0, 777), true), encoded);
        assert!(matches!(policy.mode, IdentityMode::Xino { .. }));
    }

    #[ktest]
    fn xino_encodes_fsid_in_high_bits() {
        for (fs_count, expected_shift) in [(2u64, 62u32), (3, 61), (5, 60), (9, 59)] {
            let policy = build_policy(
                XinoMode::On,
                fs_count,
                false,
                false,
                None,
                vec![numbers(3, dev(1, 1), 200)],
            );
            let resolved = match &policy.mode {
                IdentityMode::Xino { xino_shift } => *xino_shift,
                _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
            };
            assert_eq!(resolved, expected_shift);
        }

        let policy = build_policy(
            XinoMode::On,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let xino_shift = match &policy.mode {
            IdentityMode::Xino { xino_shift } => *xino_shift,
            _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
        };
        let encoded = policy.project(real_id(3, 0x1234), false);
        assert_eq!(
            encoded,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: (3 << (xino_shift + 1)) | 0x1234
            }
        );
        assert_eq!(encoded.ino >> (xino_shift + 1), 3);
        assert_eq!(encoded.ino & ((1 << xino_shift) - 1), 0x1234);
        assert_eq!(encoded.ino & (1 << xino_shift), 0);

        let auto_policy = build_policy(
            XinoMode::Auto,
            4,
            false,
            true,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        // The two mounts hold their own anonymous device, so only the ino side compares
        // across them; each device is checked against the policy that drew its pair.
        let auto_encoded = auto_policy.project(real_id(3, 0x1234), false);
        assert_eq!(auto_encoded.ino, encoded.ino);
        assert_eq!(auto_encoded.dev, auto_policy.overlay_device.id());
        assert!(matches!(auto_policy.mode, IdentityMode::Xino { .. }));
    }

    #[ktest]
    fn xino_off_or_overflow_takes_fallback() {
        let policy = build_policy(
            XinoMode::On,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let xino_shift = match &policy.mode {
            IdentityMode::Xino { xino_shift } => *xino_shift,
            _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
        };
        let oversized_ino = 1u64 << xino_shift;
        assert_eq!(
            policy.project(real_id(3, oversized_ino), false),
            ObjectVisibleId {
                dev: dev(1, 1),
                ino: oversized_ino
            }
        );
        let fallback = policy.project(real_id(3, oversized_ino), true);
        assert_eq!(
            fallback,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: (1 << xino_shift) | 1
            }
        );
        assert_ne!(fallback.ino & (1 << xino_shift), 0);
        assert_ne!(
            fallback,
            policy.project(real_id(3, oversized_ino - 1), false)
        );

        let off_policy = build_policy(
            XinoMode::Off,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        assert_eq!(
            off_policy.project(real_id(3, 7), false),
            ObjectVisibleId {
                dev: dev(1, 1),
                ino: 7
            }
        );
        assert_eq!(
            off_policy.project(real_id(3, 7), true),
            ObjectVisibleId {
                dev: off_policy.overlay_device.id(),
                ino: 1
            }
        );
    }

    #[ktest]
    fn fallback_ino_allocates_from_one() {
        let policy = build_policy(
            XinoMode::Off,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let first = policy.project(real_id(3, 7), true);
        let second = policy.project(real_id(3, 7), true);
        let third = policy.project(real_id(3, 7), true);
        assert_eq!(
            first,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 1,
            }
        );
        assert_eq!(
            second,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 2,
            }
        );
        assert_eq!(
            third,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 3,
            }
        );
    }

    #[ktest]
    fn origin_record_roundtrip_preserves_identity() {
        let row = numbers(3, dev(1, 1), 100);
        let record = ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: row.container_dev_id.as_encoded_u64(),
            root_ino: row.root_ino,
            real_ino: 0x1234,
        };
        let wire = record.as_bytes().to_vec();
        assert_eq!(wire.len(), 32);
        let decoded = ObjectOriginRecord::decode(&wire).expect("a fresh record decodes");
        assert_eq!(decoded.container_dev_id, dev(1, 1).as_encoded_u64());
        assert_eq!(decoded.root_ino, 100);
        assert_eq!(decoded.real_ino, 0x1234);

        let policy = build_policy(XinoMode::On, 4, false, false, None, vec![row]);
        assert_eq!(policy.fsid_of_origin(&decoded), Some(3));
    }

    #[ktest]
    fn origin_record_decode_rejects_malformed() {
        let mut short = valid_wire();
        short.truncate(31);
        assert!(ObjectOriginRecord::decode(&short).is_none());
        let mut long = valid_wire();
        long.push(0);
        assert!(ObjectOriginRecord::decode(&long).is_none());
        let mut word = valid_wire();
        word[0] ^= 0xff;
        assert!(ObjectOriginRecord::decode(&word).is_none());
        // The first word of a record the previous format wrote.
        let mut versioned = valid_wire();
        versioned[4] = 3;
        assert!(ObjectOriginRecord::decode(&versioned).is_none());
        let mut invalid_dev = valid_wire();
        invalid_dev[8..16]
            .copy_from_slice(&device_id::encode_device_numbers(0x1000, 0).to_ne_bytes());
        assert!(ObjectOriginRecord::decode(&invalid_dev).is_none());
    }

    #[ktest]
    fn layer_table_resolves_persistent_names() {
        let policy = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            Some(numbers(0, dev(9, 9), 900)),
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(2, 2), 200)],
        );
        assert_eq!(policy.fsid_of_origin(&record(dev(1, 1), 100, 7)), Some(1));
        assert_eq!(policy.fsid_of_origin(&record(dev(2, 2), 200, 7)), Some(2));
        assert_eq!(policy.fsid_of_origin(&record(dev(1, 1), 300, 7)), None);
        assert_eq!(policy.fsid_of_origin(&record(dev(9, 9), 900, 7)), None);

        let ambiguous = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            None,
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(1, 1), 100)],
        );
        assert_eq!(ambiguous.fsid_of_origin(&record(dev(1, 1), 100, 7)), None);

        let lower_only = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            None,
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(2, 2), 200)],
        );
        assert_eq!(lower_only.layer(1).fsid, 1);
        assert_eq!(lower_only.layer(1).root_ino, 100);
        assert_eq!(lower_only.layer(2).fsid, 2);
        assert_eq!(lower_only.layer(2).root_ino, 200);
    }

    #[ktest]
    fn origin_record_resolves_to_layer() {
        let row = numbers(3, dev(1, 1), 100);
        let policy = build_policy(
            XinoMode::On,
            5,
            false,
            false,
            None,
            vec![row, numbers(4, dev(2, 2), 200)],
        );
        let origin = ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: row.container_dev_id.as_encoded_u64(),
            root_ino: row.root_ino,
            real_ino: 0x1234,
        };
        let resolved = ObjectRealId {
            fsid: policy
                .fsid_of_origin(&origin)
                .expect("the record names one lower"),
            real_ino: origin.real_ino,
        };
        assert_eq!(resolved, real_id(3, 0x1234));
    }
}
