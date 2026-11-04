// SPDX-License-Identifier: MPL-2.0

use crate::kms::{
    objects::{
        DrmKmsObjectId, DrmKmsObjectIndex, DrmKmsObjectMask,
        property::{DrmPropertyAttachments, in_formats::DrmInFormats},
    },
    pixel_format::DrmPixelFormat,
};

/// An image layer that can be composed into a CRTC's output.
///
/// Supported CRTCs are represented by their per-type object indices in a bitmask.
#[derive(Debug)]
pub(crate) struct DrmPlane {
    id: DrmKmsObjectId,
    index: DrmKmsObjectIndex,
    config: DrmPlaneConfig,
    properties: DrmPropertyAttachments,
}

impl DrmPlane {
    pub(super) fn new(
        id: DrmKmsObjectId,
        index: DrmKmsObjectIndex,
        config: DrmPlaneConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            properties,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.id.value()
    }

    pub(crate) fn index(&self) -> DrmKmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmPlaneType {
        self.config.type_
    }

    pub(crate) fn possible_crtcs(&self) -> &DrmKmsObjectMask {
        &self.config.possible_crtcs
    }

    pub(crate) fn pixel_formats(&self) -> &[DrmPixelFormat] {
        self.config.in_formats.formats()
    }

    pub(crate) fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

/// Immutable configuration of a DRM plane.
#[derive(Debug)]
pub(super) struct DrmPlaneConfig {
    pub type_: DrmPlaneType,
    /// Each bit represents a CRTC's per-type registration index.
    pub possible_crtcs: DrmKmsObjectMask,
    pub in_formats: DrmInFormats,
}

/// The functional role of a DRM plane.
///
/// The discriminant values match the `DRM_PLANE_TYPE_*` values exposed through the standard `type` property.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_plane.h#L571-L621>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmPlaneType {
    Overlay,
    Primary,
    Cursor,
}
