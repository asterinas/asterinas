// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::time::Duration;

use aster_core::prelude::*;
use ostd::sync::Mutex;

use crate::kms::{
    display_mode::DrmDisplayMode,
    objects::{KmsObjectId, KmsObjectIndex, plane::DrmPlane, property::DrmPropertyAttachments},
    vblank::{DrmVblank, DrmVblankEvent, DrmVblankWaitResult},
};

/// A display pipeline that scans out planes using a display mode.
///
/// A CRTC owns the runtime mode and activation state of the pipeline.
/// Its primary plane is mandatory,
/// while its cursor plane is optional;
/// both links are non-owning because the KMS object store owns the topology objects.
#[derive(Debug)]
pub struct DrmCrtc {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmCrtcConfig,
    state: Mutex<DrmCrtcState>,
    properties: DrmPropertyAttachments,
    vblank: DrmVblank,
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
            vblank: DrmVblank::default(),
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
        // Serializes CRTC state changes and their vblank lifecycle notifications.
        // Lock order: CRTC state -> vblank state; vblank never locks CRTC state.
        let mut current_state = self.state.lock();
        *current_state = state;
        self.vblank.set_enabled(current_state.is_active());
    }

    pub(crate) fn primary_plane(&self) -> &Weak<DrmPlane> {
        &self.config.primary_plane
    }

    pub(crate) fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }

    pub(crate) fn vblank_sequence(&self) -> u64 {
        self.vblank.sequence()
    }

    pub(crate) fn queue_vblank_event(&self, event: DrmVblankEvent) -> Result<u64> {
        self.vblank.queue_event(event)
    }

    pub(crate) fn wait_target_sequence(&self, target_sequence: u64) -> Result<DrmVblankWaitResult> {
        self.vblank.wait_target_sequence(target_sequence)
    }

    /// Handles a device notification for one or more vblanks on this CRTC.
    ///
    /// Devices pass `sequence_delta = 1` for a normal vblank notification.
    /// Software clocks include missed vblanks in `sequence_delta` after a delay.
    /// `timestamp` is the latest vblank's monotonic timestamp.
    /// Zero increments and notifications while the CRTC is disabled or inactive
    /// are ignored.
    ///
    /// The device must call this from task context because event handling may block.
    pub fn handle_vblank(&self, timestamp: Duration, sequence_delta: u64) {
        self.vblank.handle_vblank(timestamp, sequence_delta);
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

#[derive(Clone, Debug)]
pub(crate) struct DrmCrtcState {
    enable_state: CrtcEnableState,
}

impl Default for DrmCrtcState {
    fn default() -> Self {
        Self {
            enable_state: CrtcEnableState::Disabled,
        }
    }
}

impl DrmCrtcState {
    pub(crate) fn new(display_mode: DrmDisplayMode) -> Self {
        Self {
            enable_state: CrtcEnableState::Enabled {
                display_mode,
                is_active: true,
            },
        }
    }

    pub(crate) fn display_mode(&self) -> Option<DrmDisplayMode> {
        match self.enable_state {
            CrtcEnableState::Disabled => None,
            CrtcEnableState::Enabled { display_mode, .. } => Some(display_mode),
        }
    }

    fn is_active(&self) -> bool {
        match self.enable_state {
            CrtcEnableState::Disabled => false,
            CrtcEnableState::Enabled { is_active, .. } => is_active,
        }
    }
}

/// The CRTC's enable state, requiring a mode when enabled and preventing activity when disabled.
#[derive(Clone, Debug)]
enum CrtcEnableState {
    Disabled,
    Enabled {
        display_mode: DrmDisplayMode,
        is_active: bool,
    },
}
