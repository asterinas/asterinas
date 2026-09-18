// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Mutex;

use crate::{
    kms::objects::{KmsObjectId, property::DrmPropertyAttachments},
    utils::DrmDisplayMode,
};

#[derive(Debug)]
pub struct DrmCrtc {
    gamma_size: u32,
    state: Mutex<DrmCrtcState>,
    primary_plane_id: KmsObjectId,
    cursor_plane_id: Option<KmsObjectId>,
    properties: DrmPropertyAttachments,
}

impl DrmCrtc {
    pub fn new(
        gamma_size: u32,
        primary_plane_id: KmsObjectId,
        cursor_plane_id: Option<KmsObjectId>,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            gamma_size,
            state: Mutex::new(DrmCrtcState::default()),
            primary_plane_id,
            cursor_plane_id,
            properties,
        }
    }

    pub fn gamma_size(&self) -> u32 {
        self.gamma_size
    }

    pub fn state_snapshot(&self) -> DrmCrtcState {
        self.state.lock().clone()
    }

    pub fn primary_plane_id(&self) -> KmsObjectId {
        self.primary_plane_id
    }

    pub fn cursor_plane_id(&self) -> Option<KmsObjectId> {
        self.cursor_plane_id
    }

    pub fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

#[derive(Clone, Debug, Default)]
pub struct DrmCrtcState {
    display_mode: Option<DrmDisplayMode>,
    enable: bool,
    active: bool,
}

impl DrmCrtcState {
    pub fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.display_mode
    }

    pub fn is_enabled(&self) -> bool {
        self.enable
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}
