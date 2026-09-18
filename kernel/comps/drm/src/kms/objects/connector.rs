// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Weak, vec::Vec};

use ostd::sync::Mutex;

use crate::kms::{
    display_info::DrmDisplayInfo,
    display_mode::DrmDisplayMode,
    objects::{
        KmsObjectId, KmsObjectIndex, KmsObjectMask, encoder::DrmEncoder,
        property::DrmPropertyAttachments,
    },
};

/// A display output endpoint in the DRM KMS topology.
///
/// A connector represents a physical or virtual sink such as HDMI, DisplayPort, or a writeback connector.
///
/// It records the encoders that can drive the sink, the currently selected encoder in `DrmConnectorState`, and display data discovered during probing in [`DrmConnectorProbeState`].
///
/// Each bit in `possible_encoders` corresponds to an index in the encoder registration order.
#[derive(Debug)]
pub(crate) struct DrmConnector {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmConnectorConfig,
    state: Mutex<DrmConnectorState>,
    probe_state: Mutex<DrmConnectorProbeState>,
    properties: DrmPropertyAttachments,
}

impl DrmConnector {
    pub(super) fn new(
        id: KmsObjectId,
        index: KmsObjectIndex,
        config: DrmConnectorConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmConnectorState::default()),
            probe_state: Mutex::new(DrmConnectorProbeState::default()),
            properties,
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmConnectorType {
        self.config.type_
    }

    pub(crate) fn type_index(&self) -> u32 {
        self.config.type_index
    }

    pub(crate) fn state(&self) -> &Mutex<DrmConnectorState> {
        &self.state
    }

    pub(crate) fn probe_state(&self) -> &Mutex<DrmConnectorProbeState> {
        &self.probe_state
    }

    pub(crate) fn possible_encoders(&self) -> &KmsObjectMask {
        &self.config.possible_encoders
    }

    pub fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

/// Immutable configuration of a DRM connector.
#[derive(Debug)]
pub(super) struct DrmConnectorConfig {
    pub type_: DrmConnectorType,
    pub type_index: u32,
    /// Each bit represents an encoder's per-type registration index.
    pub possible_encoders: KmsObjectMask,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmConnectorState {
    encoder: Weak<DrmEncoder>,
}

impl DrmConnectorState {
    pub(crate) fn encoder(&self) -> &Weak<DrmEncoder> {
        &self.encoder
    }
}

/// Probe-derived state of a connector.
///
/// This state records the connector's detected status, available display modes, and physical display information.
/// It is refreshed by connector probing and kept separate from the userspace-configurable `DrmConnectorState`.
#[derive(Clone, Debug)]
pub(crate) struct DrmConnectorProbeState {
    status: DrmConnectorStatus,
    display_modes: Vec<DrmDisplayMode>,
    display_info: DrmDisplayInfo,
}

impl DrmConnectorProbeState {
    pub(crate) fn status(&self) -> DrmConnectorStatus {
        self.status
    }

    pub(crate) fn display_modes(&self) -> &[DrmDisplayMode] {
        &self.display_modes
    }

    pub(crate) fn display_info(&self) -> DrmDisplayInfo {
        self.display_info
    }
}

impl Default for DrmConnectorProbeState {
    fn default() -> Self {
        Self {
            status: DrmConnectorStatus::UnknownConnection,
            display_modes: Vec::new(),
            display_info: DrmDisplayInfo::default(),
        }
    }
}

/// `DRM_MODE_CONNECTOR_X` macros in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L403-L423>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DrmConnectorType {
    Unknown = 0,
    Vga = 1,
    DviI = 2,
    DviD = 3,
    DviA = 4,
    Composite = 5,
    SVideo = 6,
    Lvds = 7,
    Component = 8,
    NinePinDin = 9,
    DisplayPort = 10,
    HdmiA = 11,
    HdmiB = 12,
    Tv = 13,
    Edp = 14,
    Virtual = 15,
    Dsi = 16,
    Dpi = 17,
    Writeback = 18,
    Spi = 19,
    Usb = 20,
}

/// `enum drm_connector_status` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_connector.h#L61-L92>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrmConnectorStatus {
    Connected = 1,
    Disconnected = 2,
    UnknownConnection = 3,
}
