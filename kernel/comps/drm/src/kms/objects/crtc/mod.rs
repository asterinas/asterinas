// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use crate::kms::objects::{
    KmsObjectId, KmsObjectIndex, OwnedKmsObjectId, crtc::vblank::DrmVblank, plane::DrmPlane,
    property::DrmPropertyAttachments,
};

pub mod vblank;

/// A display pipeline that scans out planes using a display mode.
#[derive(Debug)]
pub struct DrmCrtc {
    id: OwnedKmsObjectId,
    index: KmsObjectIndex,
    config: DrmCrtcConfig,
    properties: DrmPropertyAttachments,
    vblank: Arc<DrmVblank>,
}

impl DrmCrtc {
    pub(super) fn new(
        id: OwnedKmsObjectId,
        index: KmsObjectIndex,
        config: DrmCrtcConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            properties,
            vblank: Arc::default(),
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id.value()
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
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

    pub fn vblank(&self) -> &Arc<DrmVblank> {
        &self.vblank
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
