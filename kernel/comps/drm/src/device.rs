// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

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
    fn has_features(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }
}

bitflags::bitflags! {
    /// Capabilities provided by an Asterinas DRM device implementation.
    ///
    /// These flags are internal to the DRM subsystem. Their bit positions
    /// are not part of the DRM userspace ABI.
    pub struct DrmFeatures: u32 {
        /// Supports creation of a render device node.
        const RENDER           = 1 << 0;
        /// Supports kernel mode-setting (KMS) operations.
        const MODESET          = 1 << 1;
        /// Supports atomic mode-setting operations.
        const ATOMIC           = 1 << 2;
        /// Supports graphics execution manager (GEM) operations.
        const GEM              = 1 << 3;
        /// Supports DRM synchronization objects.
        const SYNCOBJ          = 1 << 4;
        /// Supports timeline synchronization objects.
        const SYNCOBJ_TIMELINE = 1 << 5;
        /// Requires userspace-aware cursor hotspot handling.
        const CURSOR_HOTSPOT   = 1 << 6;
    }
}
