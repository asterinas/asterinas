// SPDX-License-Identifier: MPL-2.0

//! Virtio filesystem device support.
//!
//! This module groups the virtio-fs configuration definitions, device-side
//! request handling, and DMA buffer management.

mod config;
pub mod device;
pub mod pool;

/// The default virtio-fs device name used by the kernel filesystem.
pub const DEVICE_NAME: &str = "Virtio-FS";
