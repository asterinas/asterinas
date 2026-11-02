// SPDX-License-Identifier: MPL-2.0

//! The virtio of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(custom_inner_attributes)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use alloc::{boxed::Box, sync::Arc};
use core::hint::spin_loop;

use aster_block::MajorIdOwner;
use bitflags::bitflags;
use component::{ComponentInitError, init_component};
use device::{
    VirtioDeviceError, VirtioDeviceType, block::device::BlockDevice,
    console::device::ConsoleDevice, entropy::device::EntropyDevice,
    filesystem::device::FileSystemDevice, input::device::InputDevice,
    network::device::NetworkDevice, socket::device::SocketDevice,
};
use ostd::{error, warn};
use spin::Once;
use transport::{DeviceStatus, DeviceTransport, mmio::VIRTIO_MMIO_DRIVER};

use crate::transport::VirtioTransport;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "virtio: "
    };
}

pub mod device;
mod dma_buf;
mod id_alloc;
mod queue;
mod transport;

static VIRTIO_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    VIRTIO_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());

    // Find all devices and register them to the corresponding crate
    transport::init();

    device::entropy::init();
    device::network::init();
    device::socket::init();

    while let Some(transport) = pop_device_transport() {
        let transport: Arc<dyn VirtioTransport> = Arc::from(transport);
        let device_type = transport.device_type();

        // Reset the device.
        transport
            .write_device_status(DeviceStatus::empty())
            .unwrap();
        while transport.read_device_status() != DeviceStatus::empty() {
            spin_loop();
        }

        // Reference: <https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html#x1-1230001>
        let result = (|| {
            let device_transport = DeviceTransport::new(transport.clone());
            transport.add_device_status(DeviceStatus::ACKNOWLEDGE)?;
            transport.add_device_status(DeviceStatus::DRIVER)?;
            let features = negotiate_features(transport.as_ref())?;
            if !transport.is_legacy_version() {
                transport.add_device_status(DeviceStatus::FEATURES_OK)?;
                if !transport
                    .read_device_status()
                    .contains(DeviceStatus::FEATURES_OK)
                {
                    return Err(VirtioDeviceError::UnsupportedConfig);
                }
            }

            match device_type {
                VirtioDeviceType::Block => BlockDevice::init(device_transport, features),
                VirtioDeviceType::Console => ConsoleDevice::init(device_transport, features),
                VirtioDeviceType::Entropy => EntropyDevice::init(device_transport, features),
                VirtioDeviceType::Input => InputDevice::init(device_transport, features),
                VirtioDeviceType::Network => NetworkDevice::init(device_transport, features),
                VirtioDeviceType::Socket => SocketDevice::init(device_transport, features),
                VirtioDeviceType::FileSystem => FileSystemDevice::init(device_transport, features),
                _ => {
                    warn!("Found unimplemented device: {:?}", device_type);
                    Ok(())
                }
            }
        })();
        if let Err(err) = result {
            error!(
                "Device initialization error: {:?}, device type: {:?}",
                err, device_type
            );
        }
    }
    Ok(())
}

fn pop_device_transport() -> Option<Box<dyn VirtioTransport>> {
    if let Some(device) = transport::pci::pop_device_transport() {
        return Some(device);
    }
    if let Some(device) = VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport() {
        return Some(Box::new(device));
    }
    None
}

fn negotiate_features(transport: &dyn VirtioTransport) -> Result<u64, VirtioDeviceError> {
    let features = transport.read_device_features();
    let mask = ((1u64 << 24) - 1) | (((1u64 << 24) - 1) << 50);
    let device_specified_features = features & mask;
    let device_support_features = match transport.device_type() {
        VirtioDeviceType::Network => NetworkDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Block => BlockDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Input => InputDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Console => ConsoleDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Socket => SocketDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::FileSystem => {
            FileSystemDevice::negotiate_features(device_specified_features)
        }
        _ => device_specified_features,
    };
    let mut support_feature = Feature::from_bits_truncate(features);
    support_feature.remove(Feature::RING_EVENT_IDX);
    let negotiated_features = features & (support_feature.bits() | device_support_features);
    transport.write_driver_features(negotiated_features)?;
    Ok(negotiated_features)
}

bitflags! {
    /// all device features, bits 0~23 and 50~63 are specified by device.
    /// if using this struct to translate u64, use from_bits_truncate function instead of from_bits
    ///
    #[derive(Clone, Copy, Debug)]
    struct Feature: u64 {

        // device independent
        const NOTIFY_ON_EMPTY    = 1 << 24; // legacy
        const ANY_LAYOUT         = 1 << 27; // legacy
        const RING_INDIRECT_DESC = 1 << 28;
        const RING_EVENT_IDX     = 1 << 29;
        const UNUSED             = 1 << 30; // legacy
        const VERSION_1          = 1 << 32; // detect legacy

        // since virtio v1.1
        const ACCESS_PLATFORM    = 1 << 33;
        const RING_PACKED        = 1 << 34;
        const IN_ORDER           = 1 << 35;
        const ORDER_PLATFORM     = 1 << 36;
        const SR_IOV             = 1 << 37;
        const NOTIFICATION_DATA  = 1 << 38;
        const NOTIF_CONFIG_DATA  = 1 << 39;
        const RING_RESET         = 1 << 40;
    }
}
