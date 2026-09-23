// SPDX-License-Identifier: MPL-2.0

//! Graphics Execution Manager (GEM) support.
//!
//! GEM provides DRM drivers with graphics buffer objects
//! and the core mechanisms needed to expose those objects to userspace.
//! A [`DrmGemObject`] owns a driver-provided memory backend,
//! while the DRM core manages per-file handles, mmap access,
//! and device-wide fake mmap offsets.
//!
//! A driver enables GEM by returning its [`DrmGemOps`] implementation from
//! [`DrmDevice::as_gem_ops`].
//! Drivers may use [`shmem::DrmGemShmemBackend`] for RAM-backed objects
//! or provide their own [`object::DrmGemObjectBackend`] implementation.

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::{device::DrmDevice, gem::object::DrmGemObject};

pub(crate) mod mmap_offset;
pub mod object;
pub mod shmem;

/// Operations provided by a GEM-capable DRM driver.
///
/// A device exposes these operations through [`DrmDevice::as_gem_ops`].
/// Drivers that support dumb buffers override [`DrmGemOps::create_dumb`].
pub trait DrmGemOps: Debug + DrmDevice + Send + Sync {
    /// Creates a dumb GEM buffer with the requested size.
    fn create_dumb(&self, _size: usize) -> Result<Arc<DrmGemObject>>;
}
