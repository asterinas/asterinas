// SPDX-License-Identifier: MPL-2.0

mod buffer;
mod config;
pub mod device;
pub(crate) mod driver;
mod header;

pub const DEVICE_NAME: &str = "Virtio-Net";

fn init() {
    buffer::init();
}
