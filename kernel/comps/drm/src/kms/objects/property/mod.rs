// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use aster_core::prelude::*;

use crate::kms::objects::{DrmKmsObjectType, KmsObjectId, plane::DrmPlaneType};

pub mod blob;

pub const DRM_PROP_NAME_LEN: usize = 32;
pub type KmsObjectPropValue = u64;

/// Describes a DRM property definition attached to a KMS object.
///
/// In the atomic DRM model, a property is the userspace-visible configuration
/// entry point: it defines the property's name, type, and constraints, and is
/// used to address state updates through `(object_id, property_id, value)`.
/// But a property does not, by itself, act as the kernel's single source of truth
/// for mutable state. Instead, mutable property values such as `CRTC_ID`,
/// `FB_ID`, or `SRC_X` are expected to be carried by the typed KMS object
/// state, while immutable or static properties may rely on their attached
/// value directly.
#[derive(Debug, Clone)]
pub struct DrmProperty {
    name: &'static str,
    flags: DrmPropertyFlags,
    kind: DrmPropertyKind,
}

impl DrmProperty {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn name_to_u8(&self) -> [u8; DRM_PROP_NAME_LEN] {
        str_to_u8(self.name)
    }

    pub fn flags(&self) -> &DrmPropertyFlags {
        &self.flags
    }

    pub fn kind(&self) -> &DrmPropertyKind {
        &self.kind
    }

    pub fn create(name: &'static str, flags: DrmPropertyFlags) -> Self {
        Self {
            name,
            flags,
            kind: DrmPropertyKind::Plain,
        }
    }

    pub fn create_blob(name: &'static str, flags: DrmPropertyFlags) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::BLOB,
            kind: DrmPropertyKind::Blob,
        }
    }

    pub fn create_object(
        name: &'static str,
        flags: DrmPropertyFlags,
        object_type: DrmKmsObjectType,
    ) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::OBJECT,
            kind: DrmPropertyKind::Object(object_type),
        }
    }

    pub fn create_bool(name: &'static str, flags: DrmPropertyFlags) -> Self {
        Self::create_range(name, flags, 0, 1)
    }

    pub fn create_signed_range(
        name: &'static str,
        flags: DrmPropertyFlags,
        min: i64,
        max: i64,
    ) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::SIGNED_RANGE,
            kind: DrmPropertyKind::SignedRange { min, max },
        }
    }

    pub fn create_range(name: &'static str, flags: DrmPropertyFlags, min: u64, max: u64) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::RANGE,
            kind: DrmPropertyKind::Range { min, max },
        }
    }

    pub fn create_enum(
        name: &'static str,
        flags: DrmPropertyFlags,
        enums: Vec<DrmPropertyEnum>,
    ) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::ENUM,
            kind: DrmPropertyKind::Enum(enums),
        }
    }
}

/// Stores the ordered property attachments of a KMS object.
///
/// In modern atomic DRM semantics, it should be treated primarily as the
/// userspace-facing property attachment table. Immutable or static properties
/// may rely directly on the initial value, while mutable atomic properties are
/// expected to derive their current value from the typed KMS object state. The
/// insertion order is preserved so property IDs and values can be reported to
/// userspace as corresponding arrays.
#[derive(Debug, Default)]
pub struct DrmPropertyAttachments {
    entries: Vec<DrmPropertyEntry>,
}

impl DrmPropertyAttachments {
    pub fn attach(
        &mut self,
        property_id: KmsObjectId,
        initial_value: KmsObjectPropValue,
    ) -> Result<()> {
        if self.contains(property_id) {
            return_errno_with_message!(
                Errno::EEXIST,
                "the property is already attached to the KMS object"
            );
        }

        self.entries.push(DrmPropertyEntry {
            property_id,
            initial_value,
        });
        Ok(())
    }

    pub fn entries(&self) -> &[DrmPropertyEntry] {
        &self.entries
    }

    pub fn contains(&self, property_id: KmsObjectId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.property_id == property_id)
    }

    pub fn initial_value(&self, property_id: KmsObjectId) -> Option<KmsObjectPropValue> {
        self.entries
            .iter()
            .find(|entry| entry.property_id == property_id)
            .map(|entry| entry.initial_value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A property attached to a KMS object.
#[derive(Clone, Copy, Debug)]
pub struct DrmPropertyEntry {
    property_id: KmsObjectId,
    initial_value: KmsObjectPropValue,
}

impl DrmPropertyEntry {
    pub fn property_id(&self) -> KmsObjectId {
        self.property_id
    }

    pub fn initial_value(&self) -> KmsObjectPropValue {
        self.initial_value
    }
}

/// A standard property created once for each DRM device.
///
/// This enum gives kernel code a stable, semantic way to refer to standard
/// properties without comparing their device-local object IDs or UAPI names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DrmStandardProperty {
    PlaneType,
    InFormats,
    SrcX,
    SrcY,
    SrcW,
    SrcH,
    CrtcX,
    CrtcY,
    CrtcW,
    CrtcH,
    FbId,
    CrtcId,
    Active,
    ModeId,
    Edid,
    Path,
    HdrOutputMetadata,
    Dpms,
    LinkStatus,
    NonDesktop,
    Tile,
}

impl DrmStandardProperty {
    pub(super) const ALL: [Self; 21] = [
        Self::PlaneType,
        Self::InFormats,
        Self::SrcX,
        Self::SrcY,
        Self::SrcW,
        Self::SrcH,
        Self::CrtcX,
        Self::CrtcY,
        Self::CrtcW,
        Self::CrtcH,
        Self::FbId,
        Self::CrtcId,
        Self::Active,
        Self::ModeId,
        Self::Edid,
        Self::Path,
        Self::HdrOutputMetadata,
        Self::Dpms,
        Self::LinkStatus,
        Self::NonDesktop,
        Self::Tile,
    ];

    pub(super) fn create(self) -> DrmProperty {
        match self {
            Self::PlaneType => DrmProperty::create_enum(
                "type",
                DrmPropertyFlags::IMMUTABLE,
                vec![
                    DrmPropertyEnum::new("Primary", DrmPlaneType::Primary as u64),
                    DrmPropertyEnum::new("Overlay", DrmPlaneType::Overlay as u64),
                    DrmPropertyEnum::new("Cursor", DrmPlaneType::Cursor as u64),
                ],
            ),
            Self::InFormats => DrmProperty::create_blob(
                "IN_FORMATS",
                DrmPropertyFlags::ATOMIC | DrmPropertyFlags::IMMUTABLE,
            ),
            Self::SrcX => {
                DrmProperty::create_range("SRC_X", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcY => {
                DrmProperty::create_range("SRC_Y", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcW => {
                DrmProperty::create_range("SRC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcH => {
                DrmProperty::create_range("SRC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcX => DrmProperty::create_signed_range(
                "CRTC_X",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcY => DrmProperty::create_signed_range(
                "CRTC_Y",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcW => {
                DrmProperty::create_range("CRTC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcH => {
                DrmProperty::create_range("CRTC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::FbId => DrmProperty::create_object(
                "FB_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Framebuffer,
            ),
            Self::CrtcId => DrmProperty::create_object(
                "CRTC_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Crtc,
            ),
            Self::Active => DrmProperty::create_bool("ACTIVE", DrmPropertyFlags::ATOMIC),
            Self::ModeId => DrmProperty::create_blob("MODE_ID", DrmPropertyFlags::ATOMIC),
            Self::Edid => DrmProperty::create_blob("EDID", DrmPropertyFlags::IMMUTABLE),
            Self::Path => DrmProperty::create_blob("PATH", DrmPropertyFlags::IMMUTABLE),
            Self::HdrOutputMetadata => {
                DrmProperty::create_blob("HDR_OUTPUT_METADATA", DrmPropertyFlags::ATOMIC)
            }
            Self::Dpms => DrmProperty::create_enum(
                "DPMS",
                DrmPropertyFlags::empty(),
                vec![
                    DrmPropertyEnum::new("On", 0),
                    DrmPropertyEnum::new("Standby", 1),
                    DrmPropertyEnum::new("Suspend", 2),
                    DrmPropertyEnum::new("Off", 3),
                ],
            ),
            Self::LinkStatus => DrmProperty::create_enum(
                "link-status",
                DrmPropertyFlags::empty(),
                vec![
                    DrmPropertyEnum::new("Good", 0),
                    DrmPropertyEnum::new("Bad", 1),
                ],
            ),
            Self::NonDesktop => {
                DrmProperty::create_bool("non-desktop", DrmPropertyFlags::IMMUTABLE)
            }
            Self::Tile => DrmProperty::create_blob("TILE", DrmPropertyFlags::IMMUTABLE),
        }
    }
}

bitflags::bitflags! {
    /// Property type and behavior flags exposed through the DRM property UAPI.
    ///
    /// The property-type bits describe how userspace should interpret the
    /// property's values, while [`Self::IMMUTABLE`] and [`Self::ATOMIC`]
    /// describe its behavior and visibility.
    ///
    /// Property types are mutually exclusive encodings rather than freely
    /// composable capabilities. In particular, [`Self::OBJECT`] and
    /// [`Self::SIGNED_RANGE`] use Linux's extended-type bit range.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L518-L545>.
    pub struct DrmPropertyFlags: u32 {
        /// Deprecated by Linux; retained for UAPI compatibility.
        const PENDING      = 1 << 0;
        /// An unsigned integer range property.
        const RANGE        = 1 << 1;
        /// A property that userspace cannot modify.
        const IMMUTABLE    = 1 << 2;
        /// An enumerated-value property.
        const ENUM         = 1 << 3;
        /// A property whose value is a DRM property-blob ID.
        const BLOB         = 1 << 4;
        /// A bitmask property whose named bits are described by enum entries.
        const BITMASK      = 1 << 5;
        /// A property whose value is another DRM object's ID.
        const OBJECT       = 1 << 6;
        /// A signed integer range property.
        const SIGNED_RANGE = 2 << 6;
        /// A property exposed only to clients that understand atomic KMS.
        const ATOMIC       = 0x8000_0000;
    }
}

#[derive(Debug, Clone)]
pub enum DrmPropertyKind {
    Plain,
    Range { min: u64, max: u64 },
    SignedRange { min: i64, max: i64 },
    Enum(Vec<DrmPropertyEnum>),
    Bitmask(Vec<DrmPropertyEnum>),
    Blob,
    Object(DrmKmsObjectType),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmPropertyEnum {
    name: [u8; DRM_PROP_NAME_LEN],
    value: u64,
}

impl DrmPropertyEnum {
    pub fn new(name: &'static str, value: u64) -> Self {
        Self {
            name: str_to_u8(name),
            value,
        }
    }

    pub fn name(&self) -> [u8; DRM_PROP_NAME_LEN] {
        self.name
    }

    pub fn value(&self) -> u64 {
        self.value
    }
}

fn str_to_u8(s: &str) -> [u8; DRM_PROP_NAME_LEN] {
    let mut buf = [0u8; DRM_PROP_NAME_LEN];

    let bytes = s.as_bytes();
    let len = bytes.len().min(DRM_PROP_NAME_LEN - 1);

    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}
