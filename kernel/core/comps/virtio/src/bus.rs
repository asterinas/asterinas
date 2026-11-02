// SPDX-License-Identifier: MPL-2.0

//! VirtIO devices and driver matching in the device model.
//!
//! This module defines [`VirtioBus`] and its devices, [`VirtioDevice`].
//! Transport drivers use [`device_builder()`] to create devices on this bus
//! and register them with [`aster_device::add_device()`].

use alloc::{boxed::Box, format, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_device::{
    bus::{Bus, BusDevice, BusDeviceBuilder, BusHandle},
    common::Attr,
};
use spin::Once;

use crate::{device::VirtioDeviceType, transport::VirtioTransport};

/// The bus shared by VirtIO devices across different transports.
///
/// Each device carries a `Box<dyn VirtioTransport>` as its [`Bus::Device`] payload.
/// The transport provides access to device configuration, virtqueues, and interrupts.
///
/// Drivers provide a [`VirtioDeviceType`] as their [`Bus::MatchData`]. A driver
/// matches a device when this value equals [`VirtioTransport::device_type()`].
pub struct VirtioBus;

/// A device on the VirtIO bus.
pub type VirtioDevice = BusDevice<VirtioBus>;

impl Bus for VirtioBus {
    const NAME: &'static str = "virtio";
    type Device = Box<dyn VirtioTransport>;
    type MatchData = VirtioDeviceType;

    fn matches(&self, device: &Self::Device, expected: &Self::MatchData) -> bool {
        device.device_type() == *expected
    }

    fn dev_attrs(&self) -> &'static [Attr<VirtioDevice>] {
        const ATTRS: &[Attr<VirtioDevice>] = &[
            Attr::ro("device", |dev, w| {
                writeln!(w, "0x{:04x}", dev.device_type() as u32)?;
                Ok(())
            }),
            Attr::ro("vendor", |dev, w| {
                writeln!(w, "0x{:04x}", dev.vendor_id())?;
                Ok(())
            }),
            Attr::ro("status", |dev, w| {
                writeln!(w, "0x{:08x}", dev.read_device_status().bits())?;
                Ok(())
            }),
        ];
        ATTRS
    }
}

/// Creates a builder for a VirtIO device with an automatically assigned `virtioN` name.
///
/// Use the returned builder to set the parent device, such as the PCI device
/// providing the transport. After building, call [`aster_device::add_device()`]
/// to register the device and allow matching drivers to bind to it.
///
/// # Panics
///
/// Panics if the VirtIO component has not initialized the bus.
pub fn device_builder(transport: Box<dyn VirtioTransport>) -> BusDeviceBuilder<VirtioBus> {
    let id = NEXT_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
    VirtioDevice::builder(VIRTIO_BUS.get().unwrap(), format!("virtio{id}"), transport)
}

pub(crate) static VIRTIO_BUS: Once<Arc<BusHandle<VirtioBus>>> = Once::new();

static NEXT_DEVICE_ID: AtomicUsize = AtomicUsize::new(0);
