// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use ostd::sync::Mutex;

use crate::{
    kms::{
        objects::{
            KmsObjectId, KmsObjectIndex, KmsObjectMask,
            crtc::DrmCrtc,
            framebuffer::DrmFramebuffer,
            property::{DrmPropertyAttachments, in_formats::DrmInFormats},
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
    properties: DrmPropertyAttachments,
}

impl DrmPlane {
    pub(super) fn new(
        id: KmsObjectId,
        index: KmsObjectIndex,
        config: DrmPlaneConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmPlaneState::default()),
            properties,
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

    pub(crate) fn update_state(&self, state: DrmPlaneState) {
        *self.state.lock() = state;
    }

    pub(crate) fn possible_crtcs(&self) -> &KmsObjectMask {
        &self.config.possible_crtcs
    }

    pub(crate) fn pixel_formats(&self) -> &[DrmPixelFormat] {
        self.config.in_formats.formats()
    }

    pub(crate) fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

/// Immutable configuration of a DRM plane.
#[derive(Debug)]
pub(super) struct DrmPlaneConfig {
    pub type_: DrmPlaneType,
    /// Each bit represents a CRTC's per-type registration index.
    pub possible_crtcs: KmsObjectMask,
    pub in_formats: DrmInFormats,
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
    pub(crate) fn new(
        source_rect: DrmRect,
        crtc_rect: DrmRect,
        framebuffer: Weak<DrmFramebuffer>,
        crtc: Weak<DrmCrtc>,
    ) -> Self {
        Self {
            source_rect,
            crtc_rect,
            framebuffer,
            crtc,
        }
    }

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
