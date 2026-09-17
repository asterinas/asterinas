// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Weak};

use ostd::sync::Mutex;

use crate::{
    kms::{
        objects::{
            KmsObjectId, KmsObjectIndex, KmsObjectMask, crtc::DrmCrtc, framebuffer::DrmFramebuffer,
        },
        pixel_format::DrmPixelFormat,
    },
    utils::DrmRect,
};

/// An image layer that can be composed into a CRTC's output.
///
/// A plane advertises the pixel formats and CRTCs that it supports.
/// Its runtime state selects a framebuffer, source rectangle, destination rectangle, and target CRTC.
///
/// Supported CRTCs are represented by their per-type object indices in a bitmask.
/// Runtime topology links are non-owning because the KMS object store owns both planes and CRTCs.
#[derive(Debug)]
pub(crate) struct DrmPlane {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmPlaneConfig,
    state: Mutex<DrmPlaneState>,
}

impl DrmPlane {
    pub(super) fn new(id: KmsObjectId, index: KmsObjectIndex, config: DrmPlaneConfig) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmPlaneState::default()),
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmPlaneType {
        self.config.type_
    }

    pub(crate) fn state(&self) -> &Mutex<DrmPlaneState> {
        &self.state
    }

    pub(crate) fn possible_crtcs(&self) -> &KmsObjectMask {
        &self.config.possible_crtcs
    }

    pub(crate) fn pixel_formats(&self) -> &[DrmPixelFormat] {
        &self.config.pixel_formats
    }
}

/// Immutable configuration of a DRM plane.
#[derive(Debug)]
pub(super) struct DrmPlaneConfig {
    pub type_: DrmPlaneType,
    /// Each bit represents a CRTC's per-type registration index.
    pub possible_crtcs: KmsObjectMask,
    pub pixel_formats: Box<[DrmPixelFormat]>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmPlaneState {
    // TODO: Represent source coordinates and dimensions as 16.16 fixed-point values.
    source_rect: DrmRect,
    #[expect(dead_code)]
    crtc_rect: DrmRect,

    framebuffer: Weak<DrmFramebuffer>,
    crtc: Weak<DrmCrtc>,
}

impl DrmPlaneState {
    pub(crate) fn source_rect(&self) -> DrmRect {
        self.source_rect
    }

    pub(crate) fn framebuffer(&self) -> &Weak<DrmFramebuffer> {
        &self.framebuffer
    }

    pub(crate) fn crtc(&self) -> &Weak<DrmCrtc> {
        &self.crtc
    }
}

/// The functional role of a DRM plane.
///
/// The discriminant values match the `DRM_PLANE_TYPE_*` values exposed through the standard `type` property.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_plane.h#L571-L621>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmPlaneType {
    Overlay,
    Primary,
    Cursor,
}
