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

use alloc::{sync::Arc, vec, vec::Vec};
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
            crtc::DrmCrtc,
            encoder::{DrmEncoder, DrmEncoderType},
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

const SIMPLEDRM_VREFRESH_HZ: u32 = 60;
const SIMPLEDRM_ASSUMED_DPI: u32 = 96;

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
        .with_shadow_buffer();

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

        let output_width = u32::try_from(self.boot_framebuffer.width())?;
        let output_height = u32::try_from(self.boot_framebuffer.height())?;
        if source_rect.width() != output_width || source_rect.height() != output_height {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout size does not match the physical framebuffer"
            );
        }

        let bytes_per_pixel = framebuffer.pixel_format().bytes_per_pixel();
        let row_len = self
            .boot_framebuffer
            .width()
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        if row_len > self.boot_framebuffer.line_size() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout row exceeds the physical framebuffer pitch"
            );
        }

        let pitch = framebuffer.pitch() as usize;
        let framebuffer_offset = framebuffer.offset() as usize;
        let source_x = source_rect.x() as usize;
        let source_y = source_rect.y() as usize;

        let source_x_bytes = source_x
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        let mut row = Vec::new();
        row.try_reserve_exact(row_len)
            .map_err(|_| Error::with_message(Errno::ENOMEM, "failed to allocate a scanout row"))?;
        row.resize(row_len, 0);

        for row_index in 0..self.boot_framebuffer.height() {
            let source_row = source_y.checked_add(row_index).ok_or(Errno::EOVERFLOW)?;
            let source_offset = source_row
                .checked_mul(pitch)
                .and_then(|offset| offset.checked_add(source_x_bytes))
                .and_then(|offset| offset.checked_add(framebuffer_offset))
                .ok_or(Errno::EOVERFLOW)?;
            framebuffer
                .gem_object()
                .read_bytes(source_offset, row.as_mut_slice())?;

            let destination_offset = row_index
                .checked_mul(self.boot_framebuffer.line_size())
                .ok_or(Errno::EOVERFLOW)?;
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
        let width = u32::try_from(self.boot_framebuffer.width())?;
        let height = u32::try_from(self.boot_framebuffer.height())?;
        let resolution = DrmSize::new(width, height);

        let display_mode = DrmDisplayMode::from_size(resolution, SIMPLEDRM_VREFRESH_HZ)?;
        // `simpledrm` only has the boot framebuffer's pixel geometry here, so
        // it relies on the shared physical-size fallback path.
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
        _crtc: &DrmCrtc,
        framebuffer: Option<&DrmFramebuffer>,
        source_rect: DrmRect,
        display_mode: Option<DrmDisplayMode>,
        connector_encoders: &[(Arc<DrmConnector>, Arc<DrmEncoder>)],
    ) -> Result<()> {
        let Some(display_mode) = display_mode else {
            return Ok(());
        };

        if connector_encoders.len() != 1 {
            return_errno_with_message!(
                Errno::EINVAL,
                "simpledrm requires exactly one connector for an enabled CRTC"
            );
        }

        let mode_width = display_mode.hdisplay();
        let mode_height = display_mode.vdisplay();
        if mode_width != u32::try_from(self.boot_framebuffer.width())?
            || mode_height != u32::try_from(self.boot_framebuffer.height())?
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "simpledrm only supports the physical framebuffer display mode"
            );
        }

        let framebuffer = framebuffer.ok_or(Errno::EINVAL)?;
        self.flush_framebuffer(framebuffer, source_rect)
    }

    fn dirty_fb(&self, framebuffer: &DrmFramebuffer, source_rects: &[DrmRect]) -> Result<()> {
        for source_rect in source_rects {
            self.flush_framebuffer(framebuffer, *source_rect)?;
        }

        Ok(())
    }
}
