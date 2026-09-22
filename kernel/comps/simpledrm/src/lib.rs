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

use alloc::{sync::Arc, vec};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_drm::{
    device::{DrmDevice, DrmFeatures},
    gem::{DrmGemOps, object::DrmGemObject, shmem::DrmGemShmemBackend},
    kms::{
        DrmKmsOps, DrmModeConfig,
        display_info::{DrmDisplayInfo, SubpixelOrder},
        display_mode::DrmDisplayMode,
        objects::{
            builder::DrmKmsObjectStoreBuilder,
            connector::{
                DrmConnector, DrmConnectorProbeState, DrmConnectorStatus, DrmConnectorType,
            },
            encoder::DrmEncoderType,
            plane::DrmPlaneType,
        },
        pixel_format::DrmPixelFormat,
    },
    utils::DrmSize,
};
use aster_framebuffer::{
    framebuffer::{self, FrameBuffer},
    pixel::PixelFormat,
};
use component::{ComponentInitError, init_component};

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

#[init_component(process)]
fn init() -> Result<(), ComponentInitError> {
    let Some(boot_framebuffer) = framebuffer::FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = match SimpleDrmDevice::new(boot_framebuffer) {
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
    boot_framebuffer: Arc<FrameBuffer>,
    features: DrmFeatures,
    mode_config: DrmModeConfig,
}

impl SimpleDrmDevice {
    fn new(boot_framebuffer: &Arc<FrameBuffer>) -> Result<Self> {
        let mut builder = DrmKmsObjectStoreBuilder::default();
        let pixel_format = match boot_framebuffer.pixel_format() {
            PixelFormat::BgrReserved => DrmPixelFormat::XRGB8888,
            format => {
                // TODO: Derive the exact DRM format once framebuffer initialization
                // preserves the complete boot framebuffer layout.
                // See: `kernel/core/comps/framebuffer/src/framebuffer.rs:73`.
                //
                // Until then, advertise XRGB8888 so simpledrm remains available for
                // KMS query tests. This may not match the actual framebuffer layout.
                ostd::warn!(
                    "Boot framebuffer format {:?} has no DRM mapping; assuming XRGB8888",
                    format
                );
                DrmPixelFormat::XRGB8888
            }
        };
        let primary = builder.add_plane(DrmPlaneType::Primary, vec![pixel_format])?;
        let crtc = builder.add_crtc(0, primary, None)?;
        let encoder = builder.add_encoder(DrmEncoderType::Virtual)?;
        let connector = builder.add_connector(DrmConnectorType::Virtual)?;

        builder.attach_plane_to_crtc(primary, crtc)?;
        builder.attach_encoder_to_crtc(encoder, crtc)?;
        builder.attach_connector_to_encoder(connector, encoder)?;

        let object_store = builder.build()?;

        let width = u32::try_from(boot_framebuffer.width())?;
        let height = u32::try_from(boot_framebuffer.height())?;

        let mode_config = DrmModeConfig::new(
            DrmSize::new(1, 1),
            DrmSize::new(width, height),
            object_store,
        )?
        .set_shadow_buffer();

        Ok(Self {
            boot_framebuffer: boot_framebuffer.clone(),
            features: DrmFeatures::empty(),
            mode_config,
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

    fn features(&self) -> DrmFeatures {
        self.features
    }

    fn as_gem_ops(&self) -> Option<&dyn DrmGemOps> {
        Some(self)
    }

    fn as_kms_ops(&self) -> Option<&dyn DrmKmsOps> {
        Some(self)
    }
}

impl DrmGemOps for SimpleDrmDevice {
    fn create_dumb(&self, size: usize) -> Result<Arc<DrmGemObject>> {
        DrmGemShmemBackend::new_object(size)
    }
}

impl DrmKmsOps for SimpleDrmDevice {
    fn mode_config(&self) -> &DrmModeConfig {
        &self.mode_config
    }

    fn probe_connector(&self, _connector: &DrmConnector) -> Result<DrmConnectorProbeState> {
        const SIMPLEDRM_VREFRESH_HZ: u32 = 60;
        const SIMPLEDRM_ASSUMED_DPI: u32 = 96;

        let width = u32::try_from(self.boot_framebuffer.width())?;
        let height = u32::try_from(self.boot_framebuffer.height())?;
        let resolution = DrmSize::new(width, height);

        let display_mode = DrmDisplayMode::from_size(resolution, SIMPLEDRM_VREFRESH_HZ)?;
        // `simpledrm` only has the boot framebuffer's pixel geometry here,
        // so it relies on the shared physical-size fallback path.
        let display_info =
            DrmDisplayInfo::from_dpi(resolution, SIMPLEDRM_ASSUMED_DPI, SubpixelOrder::Unknown)?;
        Ok(DrmConnectorProbeState::new(
            DrmConnectorStatus::Connected,
            vec![display_mode],
            display_info,
        ))
    }
}
