// SPDX-License-Identifier: MPL-2.0

use crate::device::{DrmDevice, DrmDeviceCaps, DrmFeatures};

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

#[derive(Debug)]
pub(crate) struct SimpleDrmDevice {
    caps: DrmDeviceCaps,
    features: DrmFeatures,
}

impl SimpleDrmDevice {
    pub fn new() -> Self {
        Self {
            caps: DrmDeviceCaps::default(),
            features: DrmFeatures::GEM | DrmFeatures::MODESET | DrmFeatures::ATOMIC,
        }
    }
}

impl DrmDevice for SimpleDrmDevice {
    fn name(&self) -> &str {
        SIMPLEDRM_NAME
    }

    fn desc(&self) -> &str {
        SIMPLEDRM_DESC
    }

    fn features(&self) -> &DrmFeatures {
        &self.features
    }

    fn caps(&self) -> &DrmDeviceCaps {
        &self.caps
    }
}
