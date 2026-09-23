// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use ostd::sync::Mutex;

use crate::kms::{
    display_mode::DrmDisplayMode,
    objects::{KmsObjectId, KmsObjectIndex, plane::DrmPlane, property::DrmPropertyAttachments},
};

/// A display pipeline that scans out planes using a display mode.
///
/// A CRTC owns the runtime mode and activation state of the pipeline.
/// Its primary plane is mandatory,
/// while its cursor plane is optional;
/// both links are non-owning because the KMS object store owns the topology objects.
#[derive(Debug)]
pub(crate) struct DrmCrtc {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmCrtcConfig,
    state: Mutex<DrmCrtcState>,
    properties: DrmPropertyAttachments,
}

impl DrmCrtc {
    pub(super) fn new(
        id: KmsObjectId,
        index: KmsObjectIndex,
        config: DrmCrtcConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmCrtcState::default()),
            properties,
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn gamma_size(&self) -> u32 {
        self.config.gamma_size
    }

    pub(crate) fn state(&self) -> &Mutex<DrmCrtcState> {
        &self.state
    }

    pub(crate) fn update_state(&self, state: DrmCrtcState) {
        *self.state.lock() = state;
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

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmCrtcState {
    display_mode: Option<DrmDisplayMode>,
    is_enabled: bool,
    #[expect(dead_code)]
    is_active: bool,
}

impl DrmCrtcState {
    pub(crate) fn new(display_mode: DrmDisplayMode) -> Self {
        Self {
            display_mode: Some(display_mode),
            is_enabled: true,
            is_active: true,
        }
    }

    pub(crate) fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.display_mode
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.is_enabled
    }
}
