// SPDX-License-Identifier: MPL-2.0

//! Kernel Mode Setting support for DRM devices.
//!
//! This module defines the shared KMS interface exposed by DRM drivers.
//! [`DrmKmsDevice`] provides device-specific KMS operations, while
//! [`DrmModeConfig`] owns the device-wide mode configuration and synchronized
//! KMS object store.
//!
//! Individual planes, CRTCs, encoders, connectors, and properties are defined
//! in the [`objects`] module.

use aster_core::prelude::*;
use ostd::sync::Mutex;

use crate::{
    device::DrmDevice,
    kms::objects::{DrmKmsObjectStore, KmsObjectId},
    utils::DrmSize,
};

pub mod objects;

/// Driver-specific operations for a KMS-capable DRM device.
///
/// A KMS-capable device exposes this interface through
/// [`DrmDevice::kms_device`].
///
/// It provides access to the device's mode configuration and implements
/// hardware-facing operations such as connector probing.
pub trait DrmKmsDevice: DrmDevice {
    fn mode_config(&self) -> &DrmModeConfig;
    fn probe_connector(&self, connector_id: KmsObjectId) -> Result<()>;
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

    pub fn with_cursor_size(mut self, cursor_size: DrmSize) -> Result<Self> {
        if !cursor_size.is_within(1..=self.max_fb_size.width(), 1..=self.max_fb_size.height()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the cursor size is empty or exceeds the maximum framebuffer size"
            );
        }

        self.cursor_size = Some(cursor_size);
        Ok(self)
    }

    pub fn with_preferred_dumb_buffer_depth(mut self, depth: u32) -> Self {
        self.preferred_dumb_buffer_depth = depth;
        self
    }

    pub fn with_async_page_flip(mut self) -> Self {
        self.supports_async_page_flip = true;
        self
    }

    pub fn with_fb_modifiers(mut self) -> Self {
        self.supports_fb_modifiers = true;
        self
    }

    pub fn with_shadow_buffer(mut self) -> Self {
        self.prefer_shadow_buffer = true;
        self
    }

    pub fn min_fb_size(&self) -> DrmSize {
        self.min_fb_size
    }

    pub fn max_fb_size(&self) -> DrmSize {
        self.max_fb_size
    }

    pub fn cursor_size(&self) -> Option<DrmSize> {
        self.cursor_size
    }

    pub fn preferred_dumb_buffer_depth(&self) -> u32 {
        self.preferred_dumb_buffer_depth
    }

    pub fn supports_async_page_flip(&self) -> bool {
        self.supports_async_page_flip
    }

    pub fn supports_fb_modifiers(&self) -> bool {
        self.supports_fb_modifiers
    }

    pub fn prefer_shadow_buffer(&self) -> bool {
        self.prefer_shadow_buffer
    }

    pub fn object_store(&self) -> &Mutex<DrmKmsObjectStore> {
        &self.object_store
    }
}
