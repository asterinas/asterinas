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
    kms::{
        DrmKmsDevice, DrmModeConfig,
        objects::{
            KmsObjectId,
            builder::DrmKmsObjectBuilder,
            connector::{DrmConnType, DrmConnectorProbeState, DrmConnectorStatus},
            encoder::DrmEncoderType,
            plane::DrmPlaneType,
        },
    },
    utils::{DrmDisplayFormat, DrmDisplayInfo, DrmDisplayMode, DrmSize, SubpixelOrder},
};
use aster_framebuffer::{
    framebuffer::{self, FrameBuffer},
    pixel::PixelFormat,
};
use component::{ComponentInitError, init_component};

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

const SIMPLEDRM_VREFRESH_HZ: u32 = 60;
const SIMPLEDRM_ASSUMED_DPI: u32 = 96;

#[init_component(process)]
fn init() -> Result<(), ComponentInitError> {
    let Some(framebuffer) = framebuffer::FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = match SimpleDrmDevice::new(framebuffer) {
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
    framebuffer: Arc<FrameBuffer>,
    features: DrmFeatures,
    mode_config: DrmModeConfig,
}

impl SimpleDrmDevice {
    fn new(framebuffer: &Arc<FrameBuffer>) -> Result<Self> {
        let mut builder = DrmKmsObjectBuilder::default();
        let format_types = match framebuffer.pixel_format() {
            PixelFormat::BgrReserved => vec![DrmDisplayFormat::XRGB8888],
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
                vec![DrmDisplayFormat::XRGB8888]
            }
        };
        let primary = builder.add_plane(DrmPlaneType::Primary, format_types);
        let crtc = builder.add_crtc(0, primary, None);
        let encoder = builder.add_encoder(DrmEncoderType::VIRTUAL);
        let connector = builder.add_connector(DrmConnType::VIRTUAL);

        builder.plane_attach_crtc(primary, crtc)?;
        builder.encoder_attach_crtc(encoder, crtc)?;
        builder.connector_attach_encoder(connector, encoder)?;

        let object_store = builder.build()?;

        let width = u32::try_from(framebuffer.width())?;
        let height = u32::try_from(framebuffer.height())?;

        let mode_config = DrmModeConfig::new(
            DrmSize::new(1, 1),
            DrmSize::new(width, height),
            object_store,
        )?
        .with_shadow_buffer();

        Ok(Self {
            framebuffer: framebuffer.clone(),
            features: DrmFeatures::MODESET,
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

    fn features(&self) -> &DrmFeatures {
        &self.features
    }

    fn kms_device(&self) -> Option<&dyn DrmKmsDevice> {
        Some(self)
    }
}

impl DrmKmsDevice for SimpleDrmDevice {
    fn mode_config(&self) -> &DrmModeConfig {
        &self.mode_config
    }

    fn probe_connector(&self, connector_id: KmsObjectId) -> Result<()> {
        let width = u32::try_from(self.framebuffer.width())?;
        let height = u32::try_from(self.framebuffer.height())?;
        let resolution = DrmSize::new(width, height);

        let display_mode = DrmDisplayMode::from_size(resolution, SIMPLEDRM_VREFRESH_HZ)?;
        // `simpledrm` only has the boot framebuffer's pixel geometry here, so
        // it relies on the shared physical-size fallback path.
        let display_info =
            DrmDisplayInfo::from_dpi(resolution, SIMPLEDRM_ASSUMED_DPI, SubpixelOrder::Unknown)?;
        let probe_state = DrmConnectorProbeState::new(
            DrmConnectorStatus::Connected,
            vec![display_mode],
            display_info,
        );

        let object_store = self.mode_config.object_store().lock();
        let connector = object_store
            .lookup_connector(connector_id)
            .ok_or(Errno::ENOENT)?;

        connector.update_probe_state(probe_state);

        Ok(())
    }
}
