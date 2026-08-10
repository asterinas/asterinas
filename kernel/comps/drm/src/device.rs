// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use crate::{DrmError, DrmRectU32};

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
///
pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn features(&self) -> &DrmFeatures;
    fn caps(&self) -> &DrmDeviceCaps;
}

impl dyn DrmDevice {
    pub fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }
}

bitflags::bitflags! {
    pub struct DrmDeviceCapFlags: u32 {
        const ASYNC_PAGE_FLIP       = 1 << 0;
        /// This field mainly exists for legacy compatibility and is the positive form of
        /// Linux `fb_modifiers_not_supported`.
        const FB_MODIFIERS          = 1 << 1;
        /// Indicates whether dumb-buffer should prefer shadow-buffer rendering.
        const SHADOW_BUFFER         = 1 << 2;
        // Blows are an Asterinas-specific capability check used by this project and
        // is not treated as a direct Linux capability query in this abstraction.
        const DUMB_BUFFER           = 1 << 3;
        const PAGE_FLIP_TARGET      = 1 << 4;
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    preferred_color_depth: u32,
    min_fb_rect: DrmRectU32,
    max_fb_rect: DrmRectU32,
    cursor_rect: DrmRectU32,

    flags: DrmDeviceCapFlags,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated geometry ranges.
    pub fn new(
        preferred_color_depth: u32,
        min_fb_rect: DrmRectU32,
        max_fb_rect: DrmRectU32,
        cursor_rect: DrmRectU32,
        flags: DrmDeviceCapFlags,
    ) -> Result<Self, DrmError> {
        if !max_fb_rect.contains_rect(&min_fb_rect) {
            return Err(DrmError::Invalid);
        }

        Ok(Self {
            preferred_color_depth,
            min_fb_rect,
            max_fb_rect,
            cursor_rect,
            flags,
        })
    }

    pub fn min_fb_rect(&self) -> DrmRectU32 {
        self.min_fb_rect
    }

    pub fn max_fb_rect(&self) -> DrmRectU32 {
        self.max_fb_rect
    }

    pub fn cursor_rect(&self) -> DrmRectU32 {
        self.cursor_rect
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
            min_fb_rect: DrmRectU32::new(0, 0, 1, 1),
            max_fb_rect: DrmRectU32::new(0, 0, 4096, 4096),
            cursor_rect: DrmRectU32::new(0, 0, 64, 64),
            // TODO: Add FLIP_TARGET after finish page_flip with target.
            flags: DrmDeviceCapFlags::DUMB_BUFFER,
        }
    }
}
