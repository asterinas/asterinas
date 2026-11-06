// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use crate::kms::objects::crtc::DrmCrtc;

#[derive(Debug, Default)]
pub(crate) struct DrmEncoderState {
    current_crtc: Weak<DrmCrtc>,
}

impl DrmEncoderState {
    pub(crate) fn new(current_crtc: Weak<DrmCrtc>) -> Self {
        Self { current_crtc }
    }

    pub(crate) fn current_crtc(&self) -> &Weak<DrmCrtc> {
        &self.current_crtc
    }
}
