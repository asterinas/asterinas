// SPDX-License-Identifier: MPL-2.0

//! The Direct Rendering Manager subsystem of Asterinas.
//!
//! This crate provides the kernel-side framework for exposing graphics devices
//! through the Linux-compatible DRM userspace API. It sits between graphics
//! devices and the character-device layer, providing the shared object model,
//! lifecycle management, permission checks, and ioctl infrastructure needed by
//! DRM devices.
//!
//! Graphics devices implement [`device::DrmDevice`] and the relevant operation
//! traits to supply hardware-specific behavior. The DRM core owns the common
//! userspace-facing semantics and coordinates access to the device, keeping
//! policy and ABI handling independent of individual device implementations.

#![no_std]
#![deny(unsafe_code)]

use alloc::sync::Arc;

use aster_core::{
    device::{Device, registry::char},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks::{self as lsm_hook, CapableContext},
};
use ostd::{sync::Mutex, task::Task};
use sparse_id_alloc::SparseIdAlloc;

use crate::{
    device::{DrmDevice, DrmFeatures, RegisteredDrmDevice},
    minor::DrmMinor,
};

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Sets this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "drm: "
    };
}

pub mod device;
mod file;
mod ioctl;
mod minor;
pub mod utils;

pub use file::{DrmFile, DrmFileCaps};
pub use minor::DrmMinorType;

fn has_current_sys_admin() -> bool {
    let task = Task::current().unwrap();
    let posix_thread = task.as_posix_thread().unwrap();

    lsm_hook::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .is_ok()
}

static DRM_DEVICE_INDEX_ALLOCATOR: Mutex<SparseIdAlloc> = Mutex::new(SparseIdAlloc::new(0, 63));

/// An index shared by all minor nodes belonging to a DRM device.
#[derive(Debug)]
struct DrmDeviceIndex(u32);

impl DrmDeviceIndex {
    fn alloc() -> Result<Self> {
        let Some(index) = DRM_DEVICE_INDEX_ALLOCATOR.lock().alloc() else {
            return_errno_with_message!(Errno::ENOMEM, "no DRM device indices are available");
        };

        Ok(Self(index))
    }

    fn index(&self) -> u32 {
        self.0
    }
}

impl Drop for DrmDeviceIndex {
    fn drop(&mut self) {
        DRM_DEVICE_INDEX_ALLOCATOR.lock().free(self.0);
    }
}

pub fn register_device(driver: Arc<dyn DrmDevice>) -> Result<()> {
    let registered_device = Arc::new(RegisteredDrmDevice::new(driver)?);

    if registered_device
        .device()
        .has_features(DrmFeatures::COMPUTE_ACCEL)
    {
        // TODO: Accel node (DRM_ACCEL) is intentionally not implemented for now.
        //
        // Rationale:
        // - The current DRM subsystem only targets primary (cardX) and render (renderDX) nodes.
        // - Modern userspace (Wayland/Mesa/Vulkan) does not rely on accel nodes.
        // - The accel minor is mainly used by specific compute-oriented drivers and is not
        //   required for virtio-gpu or basic KMS/render functionality.
        //
        // let minor = DrmMinor::new(registered_device.clone(), DrmMinorType::Accel);
        // char::register(minor).unwrap();
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "DRM acceleration devices are not supported"
        );
    }

    // TODO: Control node (controlD*) is intentionally not implemented.
    //
    // Rationale:
    // - The control minor is a legacy DRM node from the pre-KMS / early DRM model.
    // - Modern DRM userspace uses the primary node for display control and KMS, and
    //   uses the render node for rendering.
    // - There is no practical userspace dependency on a separate control node in the
    //   current Wayland/Mesa/virtio-gpu oriented design.
    //
    // let minor = DrmMinor::new(registered_device.clone(), DrmMinorType::Control);
    // char::register(minor)?;
    let render_minor = if registered_device.device().has_features(DrmFeatures::RENDER) {
        let minor = DrmMinor::new(registered_device.clone(), DrmMinorType::Render);
        char::register(minor.clone())?;
        Some(minor)
    } else {
        None
    };

    let primary_minor = DrmMinor::new(registered_device, DrmMinorType::Primary);
    if let Err(error) = char::register(primary_minor) {
        if let Some(render_minor) = render_minor {
            let _ = char::unregister(render_minor.id());
        }
        return Err(error);
    }

    Ok(())
}
