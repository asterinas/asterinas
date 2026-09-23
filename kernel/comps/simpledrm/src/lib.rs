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
            KmsObjectIndex,
            builder::DrmKmsObjectStoreBuilder,
            connector::{
                DrmConnector, DrmConnectorProbeState, DrmConnectorStatus, DrmConnectorType,
            },
            encoder::DrmEncoderType,
            framebuffer::DrmFramebuffer,
            plane::DrmPlaneType,
        },
        pixel_format::DrmPixelFormat,
    },
    utils::{DrmRect, DrmSize},
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
    pixel_format: DrmPixelFormat,
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
                // See `aster_framebuffer::framebuffer::init` for boot framebuffer initialization.
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
            pixel_format,
            mode_config,
        })
    }

    fn flush_framebuffer(&self, framebuffer: &DrmFramebuffer, source_rect: DrmRect) -> Result<()> {
        if framebuffer.pixel_format() != self.pixel_format {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM framebuffer format is not supported by simpledrm"
            );
        }

        let framebuffer_size = framebuffer.size();
        let framebuffer_rect =
            DrmRect::new(0, 0, framebuffer_size.width(), framebuffer_size.height());
        if !framebuffer_rect.contains_rect(&source_rect) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout rectangle exceeds the DRM framebuffer"
            );
        }

        let output_width = self.boot_framebuffer.width();
        let output_height = self.boot_framebuffer.height();
        if source_rect.width() as usize != output_width
            || source_rect.height() as usize != output_height
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout size does not match the physical framebuffer"
            );
        }

        let bytes_per_pixel = framebuffer.pixel_format().bytes_per_pixel();
        let row_len = output_width
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        let destination_pitch = self.boot_framebuffer.line_size();
        if row_len > destination_pitch {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout row exceeds the physical framebuffer pitch"
            );
        }

        let source_pitch = framebuffer.pitch() as usize;
        let framebuffer_offset = framebuffer.offset() as usize;
        let source_x = source_rect.x() as usize;
        let source_y = source_rect.y() as usize;

        let source_x_bytes = source_x
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        let mut row = vec![0; row_len];

        for row_index in 0..output_height {
            let source_offset =
                framebuffer_offset + (source_y + row_index) * source_pitch + source_x_bytes;
            framebuffer
                .gem_object()
                .read_bytes(source_offset, row.as_mut_slice())?;
            let destination_offset = row_index * destination_pitch;
            self.boot_framebuffer
                .write_bytes_at(destination_offset, row.as_slice())?;
        }

        Ok(())
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

    fn set_crtc(
        &self,
        _crtc_index: KmsObjectIndex,
        framebuffer: Option<&DrmFramebuffer>,
        source_rect: DrmRect,
    ) -> Result<()> {
        // simpledrm has a single CRTC backed by the boot framebuffer, so the CRTC index is unused.
        // With no framebuffer, there is nothing to copy to the firmware framebuffer.
        // simpledrm cannot turn off the firmware display, so the last image remains visible
        // while the DRM core clears its state.
        if let Some(framebuffer) = framebuffer {
            self.flush_framebuffer(framebuffer, source_rect)?;
        }
        Ok(())
    }

    fn refresh_dirty_fb(
        &self,
        framebuffer: &DrmFramebuffer,
        source_rects: &[DrmRect],
    ) -> Result<()> {
        for source_rect in source_rects {
            self.flush_framebuffer(framebuffer, *source_rect)?;
        }

        Ok(())
    }
}
