// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use ostd::sync::Mutex;

use crate::{
    kms::objects::{KmsObjectId, KmsObjectIndex, property::DrmPropertyAttachments},
    utils::{DrmDisplayInfo, DrmDisplayMode},
};

#[derive(Debug)]
pub struct DrmConnector {
    type_: DrmConnType,
    type_index: u32,
    state: Mutex<DrmConnectorState>,
    probe_state: Mutex<DrmConnectorProbeState>,
    possible_encoders: Vec<KmsObjectIndex>,
    properties: DrmPropertyAttachments,
}

impl DrmConnector {
    pub fn new(
        type_: DrmConnType,
        type_index: u32,
        possible_encoders: &[KmsObjectIndex],
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            type_,
            type_index,
            state: Mutex::new(DrmConnectorState::default()),
            probe_state: Mutex::new(DrmConnectorProbeState::default()),
            possible_encoders: possible_encoders.to_vec(),
            properties,
        }
    }

    pub fn type_(&self) -> DrmConnType {
        self.type_
    }

    pub fn type_index(&self) -> u32 {
        self.type_index
    }

    pub fn state_snapshot(&self) -> DrmConnectorState {
        self.state.lock().clone()
    }

    pub fn probe_state_snapshot(&self) -> DrmConnectorProbeState {
        self.probe_state.lock().clone()
    }

    pub fn update_probe_state(&self, probe_state: DrmConnectorProbeState) {
        *self.probe_state.lock() = probe_state;
    }

    pub fn possible_encoders(&self) -> &[KmsObjectIndex] {
        &self.possible_encoders
    }

    pub fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

#[derive(Clone, Debug, Default)]
pub struct DrmConnectorState {
    encoder_id: Option<KmsObjectId>,
}

impl DrmConnectorState {
    pub fn encoder_id(&self) -> Option<KmsObjectId> {
        self.encoder_id
    }
}

/// Probe-derived state of a connector.
///
/// This state records the connector's detected status, available display modes,
/// and physical display information.
/// It is refreshed by connector probing and kept separate from the
/// userspace-configurable [`DrmConnectorState`].
#[derive(Clone, Debug, Default)]
pub struct DrmConnectorProbeState {
    status: DrmConnectorStatus,
    display_modes: Vec<DrmDisplayMode>,
    display_info: DrmDisplayInfo,
}

impl DrmConnectorProbeState {
    pub fn new(
        status: DrmConnectorStatus,
        display_modes: Vec<DrmDisplayMode>,
        display_info: DrmDisplayInfo,
    ) -> Self {
        Self {
            status,
            display_modes,
            display_info,
        }
    }

    pub fn status(&self) -> DrmConnectorStatus {
        self.status
    }

    pub fn display_modes(&self) -> &[DrmDisplayMode] {
        &self.display_modes
    }

    pub fn display_info(&self) -> DrmDisplayInfo {
        self.display_info
    }
}

/// `macro DRM_MODE_CONNECTOR_X` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L403-L423>.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum DrmConnType {
    UNKNOWN = 0,
    VGA = 1,
    DVII = 2,
    DVID = 3,
    DVIA = 4,
    COMPOSITE = 5,
    SVIDEO = 6,
    LVDS = 7,
    COMPONENT = 8,
    _9PINDIN = 9,
    DISPLAYPORT = 10,
    HDMIA = 11,
    HDMIB = 12,
    TV = 13,
    EDP = 14,
    VIRTUAL = 15,
    DSI = 16,
    DPI = 17,
    WRITEBACK = 18,
    SPI = 19,
    USB = 20,
}

/// `enum drm_connector_status` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_connector.h#L61-L92>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DrmConnectorStatus {
    Connected = 1,
    Disconnected = 2,
    #[default]
    Unknownconnection = 3,
}
