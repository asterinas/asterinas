// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use ostd::sync::Mutex;

use crate::{
    kms::objects::{KmsObjectId, KmsObjectIndex, property::DrmPropertyAttachments},
    utils::{DrmDisplayFormat, DrmRect},
};

#[derive(Debug)]
pub struct DrmPlane {
    type_: DrmPlaneType,
    state: Mutex<DrmPlaneState>,
    possible_crtcs: Vec<KmsObjectIndex>,
    format_types: Vec<DrmDisplayFormat>,
    properties: DrmPropertyAttachments,
}

impl DrmPlane {
    pub fn new(
        type_: DrmPlaneType,
        format_types: Vec<DrmDisplayFormat>,
        possible_crtcs: &[KmsObjectIndex],
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            type_,
            state: Mutex::new(DrmPlaneState::default()),
            possible_crtcs: possible_crtcs.to_vec(),
            format_types,
            properties,
        }
    }

    pub fn type_(&self) -> DrmPlaneType {
        self.type_
    }

    pub fn state_snapshot(&self) -> DrmPlaneState {
        let state = self.state.lock();
        DrmPlaneState {
            source_rect: state.source_rect,
            crtc_rect: state.crtc_rect,
            fb_id: state.fb_id,
            crtc_id: state.crtc_id,
        }
    }

    pub fn possible_crtcs(&self) -> &[KmsObjectIndex] {
        &self.possible_crtcs
    }

    pub fn format_types(&self) -> &[DrmDisplayFormat] {
        &self.format_types
    }

    pub fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

#[derive(Debug, Default)]
pub struct DrmPlaneState {
    source_rect: DrmRect,
    crtc_rect: DrmRect,

    fb_id: Option<KmsObjectId>,
    crtc_id: Option<KmsObjectId>,
}

impl DrmPlaneState {
    pub fn source_rect(&self) -> DrmRect {
        self.source_rect
    }

    pub fn crtc_rect(&self) -> DrmRect {
        self.crtc_rect
    }

    pub fn fb_id(&self) -> Option<KmsObjectId> {
        self.fb_id
    }

    pub fn crtc_id(&self) -> Option<KmsObjectId> {
        self.crtc_id
    }
}

/// The functional role of a DRM plane.
///
/// The discriminant values match the `DRM_PLANE_TYPE_*` values exposed through
/// the standard `type` property.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_plane.h#L571-L621>.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrmPlaneType {
    Overlay,
    Primary,
    Cursor,
}
