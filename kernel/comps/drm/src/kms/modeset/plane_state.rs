// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use crate::{
    kms::objects::{crtc::DrmCrtc, framebuffer::DrmFramebuffer},
    utils::DrmRect,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmPlaneState {
    // TODO: Represent source coordinates and dimensions as 16.16 fixed-point values.
    source_rect: DrmRect,
    #[expect(dead_code)]
    crtc_rect: DrmRect,

    /// Keeps the bound framebuffer alive independently of its creating file.
    framebuffer: Option<Arc<DrmFramebuffer>>,
    crtc: Weak<DrmCrtc>,
}

impl DrmPlaneState {
    pub(crate) fn new(
        source_rect: DrmRect,
        crtc_rect: DrmRect,
        framebuffer: Option<Arc<DrmFramebuffer>>,
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

    pub(crate) fn framebuffer(&self) -> &Option<Arc<DrmFramebuffer>> {
        &self.framebuffer
    }

    pub(crate) fn crtc(&self) -> &Weak<DrmCrtc> {
        &self.crtc
    }
}
