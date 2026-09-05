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
use aster_drm::{
    DrmRect,
    device::{DrmDevice, DrmDeviceCapFlags, DrmDeviceCaps, DrmFeatures},
};
use aster_framebuffer::{
    framebuffer::{FRAMEBUFFER, FrameBuffer},
    pixel::PixelFormat,
};
use component::{ComponentInitError, init_component};

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

#[init_component(process)]
fn init() -> Result<(), ComponentInitError> {
    let Some(framebuffer) = FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init simpledrm: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = match SimpleDrmDevice::new(framebuffer.clone()) {
        Ok(device) => device,
        Err(err) => {
            ostd::warn!("Failed to create simpledrm device: {:?}", err);
            return Ok(());
        }
    };

    if let Err(err) = aster_drm::register_device(Arc::new(device)) {
        ostd::warn!("Failed to register simpledrm device: {:?}", err);
    }

    Ok(())
}

#[derive(Debug)]
struct SimpleDrmDevice {
    features: DrmFeatures,
    caps: DrmDeviceCaps,
}

impl SimpleDrmDevice {
    fn new(framebuffer: Arc<FrameBuffer>) -> Result<Self> {
        let width = u32::try_from(framebuffer.width())?;
        let height = u32::try_from(framebuffer.height())?;
        let native_rect = DrmRect::new(0, 0, width, height);
        let min_rect = DrmRect::new(0, 0, 1, 1);

        let preferred_color_depth = match framebuffer.pixel_format() {
            PixelFormat::Grayscale8 => 8,
            PixelFormat::Rgb565 => 16,
            PixelFormat::Rgb888 | PixelFormat::BgrReserved => 24,
        };

        let caps = DrmDeviceCaps::new(
            preferred_color_depth,
            min_rect,
            native_rect,
            DrmRect::default(),
            DrmDeviceCapFlags::DUMB_BUFFER,
        )?;

        Ok(Self {
            features: DrmFeatures::MODESET,
            caps,
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

    fn caps(&self) -> &DrmDeviceCaps {
        &self.caps
    }
}
