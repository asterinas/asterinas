// SPDX-License-Identifier: MPL-2.0

pub mod buffer;
pub mod config;
pub mod device;
pub mod header;

pub const DEVICE_NAME: &str = "Virtio-Net";

pub(crate) fn init() {
    buffer::init();
}
