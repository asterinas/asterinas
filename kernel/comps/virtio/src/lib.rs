// SPDX-License-Identifier: MPL-2.0

//! The virtio of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![allow(dead_code)]
#![feature(trait_alias)]
#![feature(fn_traits)]
#![feature(linked_list_cursors)]

extern crate alloc;

use alloc::boxed::Box;
use core::hint::spin_loop;

use bitflags::bitflags;
use component::{init_component, ComponentInitError};
use device::{
    block::device::BlockDevice,
    console::device::ConsoleDevice,
    input::device::InputDevice,
    network::device::NetworkDevice,
    socket::{self, device::SocketDevice},
    VirtioDeviceType,
};
use log::{error, warn};
use transport::{mmio::VIRTIO_MMIO_DRIVER, pci::VIRTIO_PCI_DRIVER, DeviceStatus};

use crate::transport::VirtioTransport;

pub mod device;
mod dma_buf;
pub mod queue;
mod transport;

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    // Find all devices and register them to the corresponding crate
    transport::init();
    // For vsock table static init
    socket::init();
    while let Some(mut transport) = pop_device_transport() {
        // Reset device
        transport
            .write_device_status(DeviceStatus::empty())
            .unwrap();
        while transport.read_device_status() != DeviceStatus::empty() {
            spin_loop();
        }

        // Set to acknowledge
        transport
            .write_device_status(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER)
            .unwrap();
        // negotiate features
        negotiate_features(&mut transport);

        if !transport.is_legacy_version() {
            // change to features ok status
            let status =
                DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::FEATURES_OK;
            transport.write_device_status(status).unwrap();
        }

        let device_type = transport.device_type();
        let res = match transport.device_type() {
            VirtioDeviceType::Block => BlockDevice::init(transport),
            VirtioDeviceType::Input => InputDevice::init(transport),
            VirtioDeviceType::Network => NetworkDevice::init(transport),
            VirtioDeviceType::Console => ConsoleDevice::init(transport),
            VirtioDeviceType::Socket => SocketDevice::init(transport),
            _ => {
                warn!("[Virtio]: Found unimplemented device:{:?}", device_type);
                Ok(())
            }
        };
        if res.is_err() {
            error!(
                "[Virtio]: Device initialization error:{:?}, device type:{:?}",
                res, device_type
            );
        }
    }
    Ok(())
}

fn pop_device_transport() -> Option<Box<dyn VirtioTransport>> {
    if let Some(device) = VIRTIO_PCI_DRIVER.get().unwrap().pop_device_transport() {
        return Some(device);
    }
    if let Some(device) = VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport() {
        return Some(Box::new(device));
    }
    None
}

fn negotiate_features(transport: &mut Box<dyn VirtioTransport>) {
    let features = transport.read_device_features();
    let mask = ((1u64 << 24) - 1) | (((1u64 << 24) - 1) << 50);
    let device_specified_features = features & mask;
    let device_support_features = match transport.device_type() {
        VirtioDeviceType::Network => NetworkDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Block => BlockDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Input => InputDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Console => ConsoleDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Socket => SocketDevice::negotiate_features(device_specified_features),
        _ => device_specified_features,
    };
    let mut support_feature = Feature::from_bits_truncate(features);
    support_feature.remove(Feature::RING_EVENT_IDX);
    transport
        .write_driver_features(features & (support_feature.bits | device_support_features))
        .unwrap();
}

bitflags! {
    /// all device features, bits 0~23 and 50~63 are specified by device.
    /// if using this struct to translate u64, use from_bits_truncate function instead of from_bits
    ///
    struct Feature: u64 {

        // device independent
        const NOTIFY_ON_EMPTY       = 1 << 24; // legacy
        const ANY_LAYOUT            = 1 << 27; // legacy
        const RING_INDIRECT_DESC    = 1 << 28;
        const RING_EVENT_IDX        = 1 << 29;
        const UNUSED                = 1 << 30; // legacy
        const VERSION_1             = 1 << 32; // detect legacy

        // since virtio v1.1
        const ACCESS_PLATFORM       = 1 << 33;
        const RING_PACKED           = 1 << 34;
        const IN_ORDER              = 1 << 35;
        const ORDER_PLATFORM        = 1 << 36;
        const SR_IOV                = 1 << 37;
        const NOTIFICATION_DATA     = 1 << 38;
        const NOTIF_CONFIG_DATA     = 1 << 39;
        const RING_RESET            = 1 << 40;
    }
}
