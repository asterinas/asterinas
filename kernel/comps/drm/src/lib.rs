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
pub mod utils;
