// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};

use aster_core::prelude::*;
use aster_util::fixed_str::FixedCStr;

use crate::kms::objects::{DrmKmsObjectType, KmsObjectId, plane::DrmPlaneType};

pub(crate) mod blob;

pub(crate) const DRM_PROP_NAME_LEN: usize = 32;
pub(super) type KmsObjectPropValue = u64;

/// A DRM property definition attached to a KMS object.
///
/// In the atomic DRM model, a property is the userspace-visible configuration
/// entry point: it defines the property's name, type, and constraints, and is
/// used to address state updates through `(object_id, property_id, value)`.
/// But a property does not, by itself, act as the kernel's single source of truth
/// for mutable state.
/// Instead, mutable property values such as `CRTC_ID`, `FB_ID`, or `SRC_X` are
/// expected to be carried by the typed KMS object state, while immutable or
/// static properties may rely on their attached value directly.
#[derive(Debug)]
pub(crate) struct DrmProperty {
    id: KmsObjectId,
    name: FixedCStr<DRM_PROP_NAME_LEN>,
    flags: DrmPropertyFlags,
    kind: DrmPropertyKind,
}

impl DrmProperty {
    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn name_to_u8(&self) -> [u8; DRM_PROP_NAME_LEN] {
        *self.name.as_array()
    }

    pub(crate) fn flags(&self) -> DrmPropertyFlags {
        self.flags
    }

    pub(crate) fn kind(&self) -> &DrmPropertyKind {
        &self.kind
    }

    #[expect(dead_code)]
    pub(super) fn new_plain(id: KmsObjectId, name: &str, flags: DrmPropertyFlags) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags,
            kind: DrmPropertyKind::Plain,
        }
    }

    fn new_blob(id: KmsObjectId, name: &str, flags: DrmPropertyFlags) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags: flags | DrmPropertyFlags::BLOB,
            kind: DrmPropertyKind::Blob,
        }
    }

    fn new_object(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        object_type: DrmKmsObjectType,
    ) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags: flags | DrmPropertyFlags::OBJECT,
            kind: DrmPropertyKind::Object(object_type),
        }
    }

    fn new_bool(id: KmsObjectId, name: &str, flags: DrmPropertyFlags) -> Self {
        Self::new_range(id, name, flags, 0, 1)
    }

    fn new_signed_range(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        min: i64,
        max: i64,
    ) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags: flags | DrmPropertyFlags::SIGNED_RANGE,
            kind: DrmPropertyKind::SignedRange { min, max },
        }
    }

    fn new_range(id: KmsObjectId, name: &str, flags: DrmPropertyFlags, min: u64, max: u64) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags: flags | DrmPropertyFlags::RANGE,
            kind: DrmPropertyKind::Range { min, max },
        }
    }

    fn new_enum(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        enums: Vec<DrmPropertyEnum>,
    ) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            flags: flags | DrmPropertyFlags::ENUM,
            kind: DrmPropertyKind::Enum(enums),
        }
    }
}

/// The ordered property attachments of a KMS object.
///
/// In modern atomic DRM semantics, it should be treated primarily as the
/// userspace-facing property attachment table.
/// Immutable or static properties may rely directly on the initial value, while
/// mutable atomic properties are expected to derive their current value from the
/// typed KMS object state.
/// The insertion order is preserved so property IDs and values can be reported
/// to userspace as corresponding arrays.
/// The object store owns each property; attachments keep weak references so
/// attaching a property does not create a second ownership graph.
#[derive(Debug, Default)]
pub struct DrmPropertyAttachments {
    entries: Vec<DrmPropertyEntry>,
}

impl DrmPropertyAttachments {
    pub(super) fn attach(
        &mut self,
        property: &Arc<DrmProperty>,
        initial_value: KmsObjectPropValue,
    ) -> Result<()> {
        if self.contains(property.id()) {
            return_errno_with_message!(
                Errno::EEXIST,
                "the property is already attached to the KMS object"
            );
        }

        self.entries.push(DrmPropertyEntry {
            property: Arc::downgrade(property),
            initial_value,
        });
        Ok(())
    }

    pub(crate) fn entries(&self) -> &[DrmPropertyEntry] {
        &self.entries
    }

    fn contains(&self, property_id: KmsObjectId) -> bool {
        self.entries
            .iter()
            .filter_map(|entry| entry.property.upgrade())
            .any(|property| property.id() == property_id)
    }
}

/// A property attached to a KMS object.
#[derive(Clone, Debug)]
pub(crate) struct DrmPropertyEntry {
    property: Weak<DrmProperty>,
    initial_value: KmsObjectPropValue,
}

impl DrmPropertyEntry {
    pub(crate) fn property(&self) -> &Weak<DrmProperty> {
        &self.property
    }

    pub(crate) fn initial_value(&self) -> KmsObjectPropValue {
        self.initial_value
    }
}

/// A standard property created once for each DRM device.
///
/// This enum gives kernel code a stable, semantic way to refer to standard
/// properties without comparing their device-local object IDs or UAPI names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum DrmStandardProperty {
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
}

impl DrmStandardProperty {
    pub(super) fn into_property(self, id: KmsObjectId) -> DrmProperty {
        match self {
            Self::PlaneType => DrmProperty::new_enum(
                id,
                "type",
                DrmPropertyFlags::IMMUTABLE,
                vec![
                    DrmPropertyEnum::new("Primary", DrmPlaneType::Primary as u64),
                    DrmPropertyEnum::new("Overlay", DrmPlaneType::Overlay as u64),
                    DrmPropertyEnum::new("Cursor", DrmPlaneType::Cursor as u64),
                ],
            ),
            Self::InFormats => DrmProperty::new_blob(id, "IN_FORMATS", DrmPropertyFlags::IMMUTABLE),
            Self::SrcX => {
                DrmProperty::new_range(id, "SRC_X", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcY => {
                DrmProperty::new_range(id, "SRC_Y", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcW => {
                DrmProperty::new_range(id, "SRC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcH => {
                DrmProperty::new_range(id, "SRC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcX => DrmProperty::new_signed_range(
                id,
                "CRTC_X",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcY => DrmProperty::new_signed_range(
                id,
                "CRTC_Y",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcW => {
                DrmProperty::new_range(id, "CRTC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcH => {
                DrmProperty::new_range(id, "CRTC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::FbId => DrmProperty::new_object(
                id,
                "FB_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Framebuffer,
            ),
            Self::CrtcId => DrmProperty::new_object(
                id,
                "CRTC_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Crtc,
            ),
            Self::Active => DrmProperty::new_bool(id, "ACTIVE", DrmPropertyFlags::ATOMIC),
            Self::ModeId => DrmProperty::new_blob(id, "MODE_ID", DrmPropertyFlags::ATOMIC),
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
    /// composable capabilities.
    /// In particular, [`Self::OBJECT`] and [`Self::SIGNED_RANGE`] use Linux's
    /// extended-type bit range.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L518-L545>.
    pub(crate) struct DrmPropertyFlags: u32 {
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

#[derive(Clone, Debug)]
pub(crate) enum DrmPropertyKind {
    Plain,
    Range {
        min: u64,
        max: u64,
    },
    SignedRange {
        min: i64,
        max: i64,
    },
    Enum(Vec<DrmPropertyEnum>),
    #[expect(dead_code)]
    Bitmask(Vec<DrmPropertyEnum>),
    Blob,
    Object(DrmKmsObjectType),
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmPropertyEnum {
    value: u64,
    name: FixedCStr<DRM_PROP_NAME_LEN>,
}

impl DrmPropertyEnum {
    fn new(name: &str, value: u64) -> Self {
        Self {
            value,
            name: FixedCStr::from_str_truncated(name),
        }
    }

    pub(crate) fn value(&self) -> u64 {
        self.value
    }
}
