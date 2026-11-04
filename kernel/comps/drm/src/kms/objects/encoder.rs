// SPDX-License-Identifier: MPL-2.0

use crate::kms::objects::{KmsObjectId, KmsObjectIndex, KmsObjectMask, OwnedKmsObjectId};

/// A hardware or virtual block that converts a CRTC's pixel stream for output.
///
/// An encoder records which CRTCs can feed it and which peer encoders may clone the same output.
#[derive(Debug)]
pub struct DrmEncoder {
    id: OwnedKmsObjectId,
    index: KmsObjectIndex,
    config: DrmEncoderConfig,
}

impl DrmEncoder {
    pub(super) fn new(
        id: OwnedKmsObjectId,
        index: KmsObjectIndex,
        config: DrmEncoderConfig,
    ) -> Self {
        Self { id, index, config }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id.value()
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmEncoderType {
        self.config.type_
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
