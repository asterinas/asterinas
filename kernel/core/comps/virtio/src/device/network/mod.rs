// SPDX-License-Identifier: MPL-2.0

mod buffer;
mod config;
pub mod device;
mod header;

pub const DEVICE_NAME: &str = "Virtio-Net";

pub(crate) fn init() {
    buffer::init();
}
