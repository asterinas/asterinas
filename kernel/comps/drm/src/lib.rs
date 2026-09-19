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
use ostd::task::Task;

use crate::{
    device::{DrmDevice, DrmFeatures, RegisteredDrmDevice},
    minor::{DrmMinor, DrmMinorType},
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

pub fn register_device(device: Arc<dyn DrmDevice>) -> Result<()> {
    let registered_device = Arc::new(RegisteredDrmDevice::new(device)?);
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
