// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use aster_core::prelude::*;

use crate::utils::DrmSize;

bitflags::bitflags! {
    pub struct DrmFeatures: u32 {
        const GEM              = 1 << 0;
        const MODESET          = 1 << 1;
        const RENDER           = 1 << 3;
        const ATOMIC           = 1 << 4;
        const SYNCOBJ          = 1 << 5;
        const SYNCOBJ_TIMELINE = 1 << 6;
        const COMPUTE_ACCEL    = 1 << 7;
        const GEM_GPUVA        = 1 << 8;
        const CURSOR_HOTSPOT   = 1 << 9;

        const USE_AGP          = 1 << 25;
        const LEGACY           = 1 << 26;
        const PCI_DMA          = 1 << 27;
        const SG               = 1 << 28;
        const HAVE_DMA         = 1 << 29;
        const HAVE_IRQ         = 1 << 30;
    }
}

/// Defines the top-level contract of a DRM device instance.
///
/// `DrmDevice` is the composition root for device-facing DRM behavior.
/// It provides stable identity metadata and shared capability discovery,
/// while higher-level DRM operations are expected to be layered as
/// dedicated operation traits.
pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn features(&self) -> &DrmFeatures;
    fn device_caps(&self) -> &DrmDeviceCaps;
}

bitflags::bitflags! {
    pub struct DrmDeviceCapFlags: u32 {
        const ASYNC_PAGE_FLIP       = 1 << 0;
        /// This field mainly exists for legacy compatibility and is the positive form of
        /// Linux `fb_modifiers_not_supported`.
        const FB_MODIFIERS          = 1 << 1;
        /// Indicates whether dumb-buffer should prefer shadow-buffer rendering.
        const PREFER_SHADOW         = 1 << 2;
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    preferred_color_depth: u32,
    min_fb_size: DrmSize,
    max_fb_size: DrmSize,
    cursor_size: Option<DrmSize>,

    flags: DrmDeviceCapFlags,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated size limits.
    pub fn new(
        preferred_color_depth: u32,
        min_fb_size: DrmSize,
        max_fb_size: DrmSize,
        cursor_size: Option<DrmSize>,
        flags: DrmDeviceCapFlags,
    ) -> Result<Self> {
        if !min_fb_size.is_within(0..=max_fb_size.width(), 0..=max_fb_size.height()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the minimum framebuffer size exceeds the maximum framebuffer size"
            );
        }

        if cursor_size
            .is_some_and(|size| !size.is_within(1..=max_fb_size.width(), 1..=max_fb_size.height()))
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the cursor size is empty or exceeds the maximum framebuffer size"
            );
        }

        Ok(Self {
            preferred_color_depth,
            min_fb_size,
            max_fb_size,
            cursor_size,
            flags,
        })
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

    pub fn preferred_color_depth(&self) -> u32 {
        self.preferred_color_depth
    }

    pub fn flags(&self) -> DrmDeviceCapFlags {
        self.flags
    }
}

impl Default for DrmDeviceCaps {
    fn default() -> Self {
        Self {
            preferred_color_depth: 24,
            min_fb_size: DrmSize::new(1, 1),
            max_fb_size: DrmSize::new(4096, 4096),
            cursor_size: None,
            flags: DrmDeviceCapFlags::empty(),
        }
    }
}
