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
use aster_device::bus::Driver;
use component::{ComponentInitError, init_component};
use device::VirtioDeviceType;
use ostd::{error, warn};
use spin::Once;
use transport::{DeviceStatus, mmio::VIRTIO_MMIO_DRIVER};

use crate::{
    bus::{VIRTIO_BUS, VirtioBus},
    driver::VirtioDriverAdapter,
};

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "virtio: "
    };
}

pub mod bus;
pub mod device;
pub mod driver;
pub mod queue;
pub mod transport;

pub use dma_buf::DmaBuf;

mod dma_buf;
mod id_alloc;

static VIRTIO_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    VIRTIO_BUS.call_once(|| aster_device::bus::register(VirtioBus).unwrap());
    transport::init();

    VIRTIO_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());
    driver::register_driver(device::block::driver::VirtioBlockDriver).unwrap();
    driver::register_driver(device::network::driver::VirtioNetDriver::new()).unwrap();
    driver::register_driver(device::console::driver::VirtioConsoleDriver).unwrap();
    driver::register_driver(device::entropy::driver::VirtioRngDriver::new()).unwrap();
    driver::register_driver(device::input::driver::VirtioInputDriver).unwrap();
    driver::register_driver(device::socket::driver::VirtioVsockDriver::new()).unwrap();
    driver::register_driver(device::filesystem::driver::VirtioFsDriver).unwrap();

    // FIXME: Remove this call once the MMIO bus is integrated into the device model.
    mmio_devices_probe();
    Ok(())
}

fn mmio_devices_probe() {
    while let Some(transport) = VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport() {
        let device = bus::device_builder(Box::new(transport)).build();
        let driver: &dyn Driver<VirtioBus> = match device.device_type() {
            VirtioDeviceType::Block => {
                &VirtioDriverAdapter(device::block::driver::VirtioBlockDriver)
            }
            VirtioDeviceType::Network => {
                &VirtioDriverAdapter(device::network::driver::VirtioNetDriver)
            }
            VirtioDeviceType::Console => {
                &VirtioDriverAdapter(device::console::driver::VirtioConsoleDriver)
            }
            VirtioDeviceType::Entropy => {
                &VirtioDriverAdapter(device::entropy::driver::VirtioRngDriver)
            }
            VirtioDeviceType::Input => {
                &VirtioDriverAdapter(device::input::driver::VirtioInputDriver)
            }
            VirtioDeviceType::Socket => {
                &VirtioDriverAdapter(device::socket::driver::VirtioVsockDriver)
            }
            VirtioDeviceType::FileSystem => {
                &VirtioDriverAdapter(device::filesystem::driver::VirtioFsDriver)
            }
            _ => {
                warn!("unsupported MMIO device type: {:?}", device.device_type());
                continue;
            }
        };
        if let Err(err) = driver.on_probe(&device) {
            error!("{} MMIO probe failed: {:?}", driver.name(), err);
        }
    }
}
