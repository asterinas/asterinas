// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use ostd::sync::Mutex;

use crate::kms::objects::{KmsObjectId, KmsObjectIndex};

#[derive(Debug)]
pub struct DrmEncoder {
    type_: DrmEncoderType,
    current_crtc_id: Mutex<Option<KmsObjectId>>,
    possible_crtcs: Vec<KmsObjectIndex>,
    possible_clones: Vec<KmsObjectIndex>,
}

impl DrmEncoder {
    pub fn new(type_: DrmEncoderType, possible_crtcs: &[KmsObjectIndex]) -> Self {
        Self {
            type_,
            current_crtc_id: Mutex::new(None),
            possible_crtcs: possible_crtcs.to_vec(),
            // TODO: Track possible encoder clones once clone compatibility is supported.
            possible_clones: Vec::new(),
        }
    }

    pub fn type_(&self) -> DrmEncoderType {
        self.type_
    }

    pub fn current_crtc_id(&self) -> Option<KmsObjectId> {
        *self.current_crtc_id.lock()
    }

    pub fn possible_crtcs(&self) -> &[KmsObjectIndex] {
        &self.possible_crtcs
    }

    pub fn possible_clones(&self) -> &[KmsObjectIndex] {
        &self.possible_clones
    }
}

/// `macro DRM_MODE_ENCODER_X` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L365-L373>.
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum DrmEncoderType {
    NONE = 0,
    DAC = 1,
    TMDS = 2,
    LVDS = 3,
    TVDAC = 4,
    VIRTUAL = 5,
    DSI = 6,
    DPMST = 7,
    DPI = 8,
}
