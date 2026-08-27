// SPDX-License-Identifier: MPL-2.0

//! A DRM driver for the bootloader-provided framebuffer.
//!
//! This component adapts the framebuffer resource from `aster-framebuffer`
//! into a DRM device. The DRM object model and userspace ABI remain outside
//! this crate.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_drm::{
    DrmError, DrmRect,
    device::{DrmDevice, DrmDeviceCapFlags, DrmDeviceCaps, DrmDeviceState, DrmFeatures},
};
use aster_framebuffer::{
    framebuffer::{FRAMEBUFFER, FrameBuffer},
    pixel::PixelFormat,
};
use component::{ComponentInitError, init_component};

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "simpledrm: "
    };
}

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let Some(framebuffer) = FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init simpledrm: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = SimpleDrmDevice::new(framebuffer.clone())?;
    aster_drm::register_drm_device(Arc::new(device));

    Ok(())
}

/// A DRM device backed by the bootloader-provided framebuffer.
#[derive(Debug)]
struct SimpleDrmDevice {
    features: DrmFeatures,
    caps: DrmDeviceCaps,
    state: DrmDeviceState,
}

impl SimpleDrmDevice {
    fn new(framebuffer: Arc<FrameBuffer>) -> Result<Self, DrmError> {
        let width = u32::try_from(framebuffer.width()).map_err(|_| DrmError::Invalid)?;
        let height = u32::try_from(framebuffer.height()).map_err(|_| DrmError::Invalid)?;
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
            state: DrmDeviceState::default(),
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

    fn state(&self) -> &DrmDeviceState {
        &self.state
    }
}
