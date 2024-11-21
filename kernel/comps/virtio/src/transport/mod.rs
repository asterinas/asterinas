// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;
use core::fmt::Debug;

use aster_util::safe_ptr::SafePtr;
use ostd::{
    arch::device::io_port::{PortRead, PortWrite},
    bus::pci::cfg_space::Bar,
    io_mem::IoMem,
    mm::{DmaCoherent, PodOnce},
    trap::IrqCallbackFunction,
    Pod,
};

use self::{mmio::virtio_mmio_init, pci::virtio_pci_init};
use crate::{
    queue::{AvailRing, Descriptor, UsedRing},
    VirtioDeviceType,
};

pub mod mmio;
pub mod pci;

/// The transport of virtio device. Virtio device can use this transport to:
/// 1. Set device status.
/// 2. Negotiate features.
/// 3. Access device config memory.
/// 4. Config virtqueue.
/// 5. Get the interrupt resources allocated to the device.
pub trait VirtioTransport: Sync + Send + Debug {
    // ====================Device related APIs=======================

    /// Get device type.
    fn device_type(&self) -> VirtioDeviceType;

    /// Get device features.
    fn read_device_features(&self) -> u64;

    /// Set driver features.
    fn write_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError>;

    /// Get device status.
    fn read_device_status(&self) -> DeviceStatus;

    /// Set device status.
    fn write_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError>;

    // Set to driver ok status
    fn finish_init(&mut self) {
        self.write_device_status(
            DeviceStatus::ACKNOWLEDGE
                | DeviceStatus::DRIVER
                | DeviceStatus::FEATURES_OK
                | DeviceStatus::DRIVER_OK,
        )
        .unwrap();
    }

    /// Get access to the device config memory.
    fn device_config_mem(&self) -> Option<IoMem>;

    /// Get access to the device config BAR space.
    fn device_config_bar(&self) -> Option<(Bar, usize)>;

    // ====================Virtqueue related APIs====================

    /// Get the total number of queues
    fn num_queues(&self) -> u16;

    /// Set virtqueue information. Some transport may set other necessary information such as MSI-X vector in PCI transport.
    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, DmaCoherent>,
        avail_ring_ptr: &SafePtr<AvailRing, DmaCoherent>,
        used_ring_ptr: &SafePtr<UsedRing, DmaCoherent>,
    ) -> Result<(), VirtioTransportError>;

    /// The max queue size of one virtqueue.
    fn max_queue_size(&self, idx: u16) -> Result<u16, VirtioTransportError>;

    /// Get notify manager of a virtqueue. User should send notification (e.g. write 0 to the pointer)
    /// after it add buffers into the corresponding virtqueue.
    fn notify_config(&self, idx: usize) -> ConfigManager<u32>;

    fn is_legacy_version(&self) -> bool;

    // ====================Device interrupt APIs=====================

    /// Registers a callback for queue interrupts.
    ///
    /// If `single_interrupt` is enabled, the transport will initially
    /// attempt to allocate a single IRQ line for the callback.
    /// If no available IRQ lines are found for allocation, the
    /// transport may assign the callback to a shared IRQ line.
    fn register_queue_callback(
        &mut self,
        index: u16,
        func: Box<IrqCallbackFunction>,
        single_interrupt: bool,
    ) -> Result<(), VirtioTransportError>;

    /// Register configuration space change interrupt callback.
    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError>;
}

/// Manage PCI device/notify configuration space (legacy/modern).
#[derive(Debug)]
pub struct ConfigManager<T: Pod> {
    modern_space: Option<SafePtr<T, IoMem>>,
    legacy_space: Option<(Bar, usize)>,
}

impl<T: Pod> ConfigManager<T> {
    pub(super) fn new(
        modern_space: Option<SafePtr<T, IoMem>>,
        legacy_space: Option<(Bar, usize)>,
    ) -> Self {
        Self {
            modern_space,
            legacy_space,
        }
    }

    /// Return if the modern configuration space exists.
    pub(super) fn is_modern(&self) -> bool {
        self.modern_space.is_some()
    }

    fn read_modern<V: PodOnce + PortRead>(&self, offset: usize) -> Result<V, VirtioTransportError> {
        let Some(safe_ptr) = self.modern_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        let field_ptr: SafePtr<V, &IoMem> = {
            let mut ptr = safe_ptr.borrow_vm();
            ptr.byte_add(offset);
            ptr.cast()
        };

        field_ptr
            .read_once()
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    fn read_legacy<V: PodOnce + PortRead>(&self, offset: usize) -> Result<V, VirtioTransportError> {
        let Some((bar, base)) = self.legacy_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        bar.read_once(base + offset)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    /// Read a specific configuration.
    pub(super) fn read_once<V: PodOnce + PortRead>(
        &self,
        offset: usize,
    ) -> Result<V, VirtioTransportError> {
        debug_assert!(offset + size_of::<V>() <= size_of::<T>());
        if self.is_modern() {
            self.read_modern(offset)
        } else {
            self.read_legacy(offset)
        }
    }

    fn write_modern<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        let Some(safe_ptr) = self.modern_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        let field_ptr: SafePtr<V, &IoMem> = {
            let mut ptr = safe_ptr.borrow_vm();
            ptr.byte_add(offset);
            ptr.cast()
        };

        field_ptr
            .write_once(&value)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    fn write_legacy<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        let Some((bar, base)) = self.legacy_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        bar.write_once(base + offset, value)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    /// Write a specific configuration.
    pub(super) fn write_once<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        debug_assert!(offset + size_of::<V>() <= size_of::<T>());
        if self.is_modern() {
            self.write_modern(offset, value)
        } else {
            self.write_legacy(offset, value)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VirtioTransportError {
    DeviceStatusError,
    InvalidArgs,
    NotEnoughResources,
}

bitflags::bitflags! {
    /// The device status field.
    pub struct DeviceStatus: u8 {
        /// Indicates that the guest OS has found the device and recognized it
        /// as a valid virtio device.
        const ACKNOWLEDGE = 1;

        /// Indicates that the guest OS knows how to drive the device.
        const DRIVER = 2;

        /// Indicates that something went wrong in the guest, and it has given
        /// up on the device. This could be an internal error, or the driver
        /// didn’t like the device for some reason, or even a fatal error
        /// during device operation.
        const FAILED = 128;

        /// Indicates that the driver has acknowledged all the features it
        /// understands, and feature negotiation is complete.
        const FEATURES_OK = 8;

        /// Indicates that the driver is set up and ready to drive the device.
        const DRIVER_OK = 4;

        /// Indicates that the device has experienced an error from which it
        /// can’t recover.
        const DEVICE_NEEDS_RESET = 64;
    }
}

pub fn init() {
    virtio_pci_init();
    virtio_mmio_init();
}
