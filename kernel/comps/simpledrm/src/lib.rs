// SPDX-License-Identifier: MPL-2.0

//! A simpledrm driver backed by the bootloader-provided framebuffer.
//!
//! It obtains framebuffer information from `aster-framebuffer` and registers
//! the resulting DRM device with `aster-drm`.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "simpledrm: "
    };
}

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_drm::device::{DrmDevice, DrmFeatures};
use aster_framebuffer::framebuffer::{self, FrameBuffer};
use component::{ComponentInitError, init_component};

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

#[init_component(process)]
fn init() -> Result<(), ComponentInitError> {
    let Some(framebuffer) = framebuffer::FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = match SimpleDrmDevice::new(framebuffer.clone()) {
        Ok(device) => device,
        Err(err) => {
            ostd::warn!("Failed to create device: {:?}", err);
            return Ok(());
        }
    };

    if let Err(err) = aster_drm::register_device(Arc::new(device)) {
        ostd::warn!("Failed to register device: {:?}", err);
    }

    Ok(())
}

#[derive(Debug)]
struct SimpleDrmDevice {
    features: DrmFeatures,
}

impl SimpleDrmDevice {
    fn new(_framebuffer: Arc<FrameBuffer>) -> Result<Self> {
        Ok(Self {
            features: DrmFeatures::MODESET,
        })
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
}
