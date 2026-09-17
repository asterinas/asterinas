// SPDX-License-Identifier: MPL-2.0

//! Kernel Mode Setting support for DRM devices.
//!
//! This module defines the shared KMS interface exposed by DRM drivers.
//! [`DrmKmsOps`] provides device-specific KMS operations,
//! while [`DrmModeConfig`] owns the device-wide mode configuration and synchronized KMS object store.
//!
//! Individual planes, CRTCs, encoders, connectors, and properties are defined in the [`objects`] module.

use aster_core::prelude::*;
use ostd::sync::Mutex;

use crate::{device::DrmDevice, kms::objects::DrmKmsObjectStore, utils::DrmSize};

pub mod display_info;
pub mod display_mode;
pub mod objects;
pub mod pixel_format;

/// Driver-specific operations for a KMS-capable DRM device.
///
/// A KMS-capable device exposes this interface through [`DrmDevice::as_kms_ops`].
///
/// It provides access to the device's mode configuration and implements hardware-facing operations
/// such as connector probing.
pub trait DrmKmsOps: DrmDevice {
    fn mode_config(&self) -> &DrmModeConfig;
}

/// Describes a DRM device's global mode-setting capabilities and KMS objects.
#[derive(Debug)]
pub struct DrmModeConfig {
    min_fb_size: DrmSize,
    max_fb_size: DrmSize,
    cursor_size: Option<DrmSize>,
    preferred_dumb_buffer_depth: u32,

    supports_async_page_flip: bool,
    supports_fb_modifiers: bool,
    prefer_shadow_buffer: bool,

    object_store: Mutex<DrmKmsObjectStore>,
}

impl DrmModeConfig {
    pub fn new(
        min_fb_size: DrmSize,
        max_fb_size: DrmSize,
        object_store: DrmKmsObjectStore,
    ) -> Result<Self> {
        if !min_fb_size.is_within(0..=max_fb_size.width(), 0..=max_fb_size.height()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the minimum framebuffer size exceeds the maximum framebuffer size"
            );
        }

        Ok(Self {
            min_fb_size,
            max_fb_size,
            cursor_size: None,
            preferred_dumb_buffer_depth: 0,
            supports_async_page_flip: false,
            supports_fb_modifiers: false,
            prefer_shadow_buffer: false,
            object_store: Mutex::new(object_store),
        })
    }

    /// Declares the fixed dimensions supported by hardware cursor planes.
    ///
    /// The cursor dimensions must be nonzero and fit within the device's maximum framebuffer dimensions.
    pub fn set_cursor_size(mut self, cursor_size: DrmSize) -> Result<Self> {
        if !cursor_size.is_within(1..=self.max_fb_size.width(), 1..=self.max_fb_size.height()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the cursor size is empty or exceeds the maximum framebuffer size"
            );
        }

        self.cursor_size = Some(cursor_size);
        Ok(self)
    }

    /// Sets the driver's preferred bit depth for dumb buffers.
    ///
    /// This records the device policy used when choosing a basic scanout buffer format.
    pub fn set_preferred_dumb_buffer_depth(mut self, depth: u32) -> Self {
        self.preferred_dumb_buffer_depth = depth;
        self
    }

    /// Advertises support for asynchronous page flips.
    ///
    /// Userspace can discover this capability through the DRM capability query interface.
    pub fn set_async_page_flip(mut self) -> Self {
        self.supports_async_page_flip = true;
        self
    }

    /// Advertises support for framebuffer format modifiers.
    ///
    /// Userspace can then use modifier-aware framebuffer creation ioctls.
    pub fn set_fb_modifiers(mut self) -> Self {
        self.supports_fb_modifiers = true;
        self
    }

    /// Marks that scanout should prefer a CPU-accessible shadow buffer.
    ///
    /// This is appropriate when rendering directly into the scanout buffer
    /// is inefficient or unsupported by the device.
    pub fn set_shadow_buffer(mut self) -> Self {
        self.prefer_shadow_buffer = true;
        self
    }

    pub(crate) fn min_fb_size(&self) -> DrmSize {
        self.min_fb_size
    }

    pub(crate) fn max_fb_size(&self) -> DrmSize {
        self.max_fb_size
    }

    pub(crate) fn cursor_size(&self) -> Option<DrmSize> {
        self.cursor_size
    }

    pub(crate) fn preferred_dumb_buffer_depth(&self) -> u32 {
        self.preferred_dumb_buffer_depth
    }

    pub(crate) fn supports_async_page_flip(&self) -> bool {
        self.supports_async_page_flip
    }

    pub(crate) fn supports_fb_modifiers(&self) -> bool {
        self.supports_fb_modifiers
    }

    pub(crate) fn prefer_shadow_buffer(&self) -> bool {
        self.prefer_shadow_buffer
    }

    pub(crate) fn object_store(&self) -> &Mutex<DrmKmsObjectStore> {
        &self.object_store
    }
}
