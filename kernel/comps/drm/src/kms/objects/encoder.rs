// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use ostd::sync::Mutex;

use crate::kms::objects::{KmsObjectId, KmsObjectIndex, KmsObjectMask, crtc::DrmCrtc};

/// A hardware or virtual block that converts a CRTC's pixel stream for output.
///
/// An encoder records which CRTCs can feed it and which peer encoders may clone the same output.
///
/// Its current CRTC is mutable routing state,
/// while the compatibility masks use per-type object indices to describe the device's fixed KMS topology.
#[derive(Debug)]
pub struct DrmEncoder {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmEncoderConfig,
    current_crtc: Mutex<Weak<DrmCrtc>>,
}

impl DrmEncoder {
    pub(super) fn new(id: KmsObjectId, index: KmsObjectIndex, config: DrmEncoderConfig) -> Self {
        Self {
            id,
            index,
            config,
            current_crtc: Mutex::new(Weak::new()),
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmEncoderType {
        self.config.type_
    }

    pub(crate) fn current_crtc(&self) -> Weak<DrmCrtc> {
        self.current_crtc.lock().clone()
    }

    pub(crate) fn set_current_crtc(&self, crtc: Weak<DrmCrtc>) {
        *self.current_crtc.lock() = crtc;
    }

    pub(crate) fn possible_crtcs(&self) -> &KmsObjectMask {
        &self.config.possible_crtcs
    }

    pub(crate) fn possible_clones(&self) -> &KmsObjectMask {
        &self.config.possible_clones
    }
}

/// Immutable configuration of a DRM encoder.
#[derive(Debug)]
pub(super) struct DrmEncoderConfig {
    pub type_: DrmEncoderType,
    /// Each bit represents a CRTC's per-type registration index.
    pub possible_crtcs: KmsObjectMask,
    /// Each bit represents an encoder's per-type registration index.
    pub possible_clones: KmsObjectMask,
}

/// `macro DRM_MODE_ENCODER_X` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L365-L373>.
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum DrmEncoderType {
    None = 0,
    Dac = 1,
    Tmds = 2,
    Lvds = 3,
    TvDac = 4,
    Virtual = 5,
    Dsi = 6,
    DpMst = 7,
    Dpi = 8,
}
