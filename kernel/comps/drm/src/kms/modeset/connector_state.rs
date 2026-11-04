// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use crate::kms::objects::encoder::DrmEncoder;

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmConnectorState {
    encoder: Weak<DrmEncoder>,
}

impl DrmConnectorState {
    pub(crate) fn new(encoder: Weak<DrmEncoder>) -> Self {
        Self { encoder }
    }

    pub(crate) fn encoder(&self) -> &Weak<DrmEncoder> {
        &self.encoder
    }
}
