// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use crate::kms::objects::{crtc::DrmCrtc, encoder::DrmEncoder};

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmConnectorState {
    crtc: Weak<DrmCrtc>,
    encoder: Weak<DrmEncoder>,
}

impl DrmConnectorState {
    pub(crate) fn new(crtc: Weak<DrmCrtc>, encoder: Weak<DrmEncoder>) -> Self {
        Self { crtc, encoder }
    }

    pub(crate) fn crtc(&self) -> &Weak<DrmCrtc> {
        &self.crtc
    }

    pub(crate) fn encoder(&self) -> &Weak<DrmEncoder> {
        &self.encoder
    }
}
