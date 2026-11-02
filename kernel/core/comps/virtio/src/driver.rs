// SPDX-License-Identifier: MPL-2.0

//! Functional drivers and their common VirtIO initialization protocol.

use alloc::{boxed::Box, sync::Arc};
use core::hint::spin_loop;

use aster_device::{
    bus::{Driver, DriverHandle},
    common::{self, Attr},
};
use bitflags::bitflags;
use inherit_methods_macro::inherit_methods;

use crate::{
    bus::{VIRTIO_BUS, VirtioBus, VirtioDevice},
    device::{VirtioDeviceError, VirtioDeviceType},
    transport::{DeviceStatus, VirtioTransport},
};

/// A functional driver for one VirtIO device type.
pub trait VirtioDriver: Send + Sync + 'static {
    /// Returns the driver directory name under `/sys/bus/virtio/drivers`.
    fn name(&self) -> &str;

    /// Returns the device type accepted by this driver.
    fn match_data(&self) -> &VirtioDeviceType;

    /// Selects the supported subset of the offered device-specific feature bits.
    ///
    /// Common transport and queue features are selected by the adapter.
    fn negotiate_features(&self, offered: u64) -> u64;

    /// Initializes and registers the device using the negotiated features.
    ///
    /// The adapter has reset the device and completed feature negotiation.
    /// The driver sets `DRIVER_OK` after preparing its queues and callbacks, before
    /// notifying queues or publishing the device for I/O.
    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError>;

    /// Returns additional attributes exposed after successful binding.
    ///
    /// Removal and rollback after attribute installation fails are not supported yet.
    fn dev_attrs(&self) -> &'static [Attr<VirtioDevice>] {
        &[]
    }
}

/// Registers a functional driver and probes matching devices already on the bus.
///
/// The low-level VirtIO component must have initialized the bus first.
pub fn register_driver<D: VirtioDriver>(driver: D) -> common::Result<Arc<DriverHandle<VirtioBus>>> {
    VIRTIO_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(VirtioDriverAdapter(driver)))
}

pub(super) struct VirtioDriverAdapter<D>(pub(super) D);

#[inherit_methods(from = "self.0")]
impl<D: VirtioDriver> Driver<VirtioBus> for VirtioDriverAdapter<D> {
    fn name(&self) -> &str;

    fn match_data(&self) -> &VirtioDeviceType;

    fn dev_attrs(&self) -> &'static [Attr<VirtioDevice>];

    fn on_probe(&self, device: &Arc<VirtioDevice>) -> common::Result<()> {
        let transport = device.payload().as_ref();
        // Reset the device.
        transport
            .write_device_status(DeviceStatus::empty())
            .unwrap();
        while transport.read_device_status() != DeviceStatus::empty() {
            spin_loop();
        }
        transport.clear_callbacks();

        // Reference: <https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html#x1-1230001>
        let result = (|| {
            transport.add_device_status(DeviceStatus::ACKNOWLEDGE)?;
            transport.add_device_status(DeviceStatus::DRIVER)?;

            let negotiated = self.negotiate_features(transport)?;
            if !transport.is_legacy_version() {
                transport.add_device_status(DeviceStatus::FEATURES_OK)?;
                if !transport
                    .read_device_status()
                    .contains(DeviceStatus::FEATURES_OK)
                {
                    return Err(VirtioDeviceError::UnsupportedConfig);
                }
            }

            self.0.probe(device.clone(), negotiated)
        })();

        result
            .or_else(|err| {
                transport.clear_callbacks();
                transport.add_device_status(DeviceStatus::FAILED)?;
                Err(err)
            })
            .map_err(|err: VirtioDeviceError| {
                ostd::error!("{} probe failed: {:?}", self.name(), err);
                common::Error::ProbeFailed
            })
    }

    fn on_release(&self, _device: &Arc<VirtioDevice>) {
        // FIXME: Stop I/O and unregister functional devices before releasing their resources.
        panic!("virtio driver removal is not supported");
    }

    fn suppress_bind_attrs(&self) -> bool {
        true
    }
}

impl<D: VirtioDriver> VirtioDriverAdapter<D> {
    /// Negotiates device and common features before the functional driver is probed.
    fn negotiate_features(
        &self,
        transport: &dyn VirtioTransport,
    ) -> Result<u64, VirtioDeviceError> {
        let features = transport.read_device_features();
        let mask = ((1u64 << 24) - 1) | (((1u64 << 24) - 1) << 50);
        let device_specified_features = features & mask;
        let device_support_features = self.0.negotiate_features(device_specified_features);
        let mut support_feature = Feature::from_bits_truncate(features);
        support_feature.remove(Feature::RING_EVENT_IDX);
        let negotiated_features = features & (support_feature.bits() | device_support_features);
        transport.write_driver_features(negotiated_features)?;
        Ok(negotiated_features)
    }
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
