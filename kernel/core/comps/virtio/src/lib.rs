// SPDX-License-Identifier: MPL-2.0

//! The virtio of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(custom_inner_attributes)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use alloc::boxed::Box;

use aster_block::MajorIdOwner;
use component::{ComponentInitError, init_component};
use device::VirtioDeviceType;
use ostd::error;
use spin::Once;
use transport::{DeviceStatus, mmio::VIRTIO_MMIO_DRIVER};

use crate::bus::VirtioBus;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "virtio: "
    };
}

mod bus;
pub mod device;
mod dma_buf;
mod driver;
mod id_alloc;
mod queue;
mod transport;

static VIRTIO_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    VIRTIO_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());

    bus::VIRTIO_BUS.call_once(|| aster_device::bus::register(VirtioBus).unwrap());
    transport::init();

    driver::register_driver(device::block::driver::VirtioBlockDriver).unwrap();
    driver::register_driver(device::network::driver::VirtioNetDriver::new()).unwrap();
    driver::register_driver(device::console::driver::VirtioConsoleDriver).unwrap();
    driver::register_driver(device::entropy::driver::VirtioRngDriver).unwrap();
    driver::register_driver(device::input::driver::VirtioInputDriver).unwrap();
    driver::register_driver(device::socket::driver::VirtioVsockDriver::new()).unwrap();
    driver::register_driver(device::filesystem::driver::VirtioFsDriver).unwrap();

    register_mmio_devices();
    Ok(())
}

fn register_mmio_devices() {
    while let Some(transport) = VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport() {
        // FIXME: Set the parent once MMIO devices are part of the device model.
        let device = bus::device_builder(Box::new(transport)).build();
        if let Err(err) = aster_device::add_device(&device) {
            error!(
                "failed to register MMIO device {:?}: {:?}",
                device.device_type(),
                err
            );
        }
    }
}
