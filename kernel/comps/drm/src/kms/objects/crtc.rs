// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use crate::kms::objects::{
    DrmKmsObjectId, DrmKmsObjectIndex, plane::DrmPlane, property::DrmPropertyAttachments,
};

/// A display pipeline that scans out planes using a display mode.
#[derive(Debug)]
pub(crate) struct DrmCrtc {
    id: DrmKmsObjectId,
    index: DrmKmsObjectIndex,
    config: DrmCrtcConfig,
    properties: DrmPropertyAttachments,
}

impl DrmCrtc {
    pub(super) fn new(
        id: DrmKmsObjectId,
        index: DrmKmsObjectIndex,
        config: DrmCrtcConfig,
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

    pub(crate) fn gamma_size(&self) -> u32 {
        self.config.gamma_size
    }

    pub(crate) fn primary_plane(&self) -> &Weak<DrmPlane> {
        &self.config.primary_plane
    }

    pub(crate) fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

/// Immutable configuration of a DRM CRTC.
#[derive(Debug)]
pub(super) struct DrmCrtcConfig {
    pub gamma_size: u32,
    pub primary_plane: Weak<DrmPlane>,
    #[expect(dead_code)]
    pub cursor_plane: Weak<DrmPlane>,
}
