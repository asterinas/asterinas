// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use ostd::sync::Mutex;

use crate::kms::{
    display_mode::DrmDisplayMode,
    objects::{KmsObjectId, KmsObjectIndex, plane::DrmPlane},
};

/// A display pipeline that scans out planes using a display mode.
///
/// A CRTC owns the runtime mode and activation state of the pipeline.
/// Its primary plane is mandatory, while its cursor plane is optional;
/// both links are non-owning because the KMS object store owns the topology
/// objects.
#[derive(Debug)]
pub(crate) struct DrmCrtc {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmCrtcConfig,
    state: Mutex<DrmCrtcState>,
}

impl DrmCrtc {
    pub(super) fn new(id: KmsObjectId, index: KmsObjectIndex, config: DrmCrtcConfig) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmCrtcState::default()),
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

    pub(crate) fn state_snapshot(&self) -> DrmCrtcState {
        self.state.lock().clone()
    }

    pub(crate) fn primary_plane(&self) -> &Weak<DrmPlane> {
        &self.config.primary_plane
    }

    pub(crate) fn cursor_plane(&self) -> &Weak<DrmPlane> {
        &self.config.cursor_plane
    }
}

/// Immutable configuration of a DRM CRTC.
#[derive(Debug)]
pub(super) struct DrmCrtcConfig {
    pub gamma_size: u32,
    pub primary_plane: Weak<DrmPlane>,
    pub cursor_plane: Weak<DrmPlane>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmCrtcState {
    display_mode: Option<DrmDisplayMode>,
    is_enabled: bool,
    is_active: bool,
}

impl DrmCrtcState {
    pub(crate) fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.display_mode
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub(crate) fn is_active(&self) -> bool {
        self.is_active
    }
}
