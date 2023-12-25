use core::fmt::Debug;

use alloc::boxed::Box;
use aster_frame::{io_mem::IoMem, trap::IrqCallbackFunction, vm::DmaCoherent};
use aster_util::safe_ptr::SafePtr;

use crate::{
    queue::{AvailRing, Descriptor, UsedRing},
    VirtioDeviceType,
};

use self::{mmio::virtio_mmio_init, pci::virtio_pci_init};

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

    fn device_type(&self) -> VirtioDeviceType;

    /// Get device features.
    fn device_features(&self) -> u64;

    /// Set driver features.
    fn set_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError>;

    /// Get device status.
    fn device_status(&self) -> DeviceStatus;

    /// Set device status.
    fn set_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError>;

    // Set to driver ok status
    fn finish_init(&mut self) {
        self.set_device_status(
            DeviceStatus::ACKNOWLEDGE
                | DeviceStatus::DRIVER
                | DeviceStatus::FEATURES_OK
                | DeviceStatus::DRIVER_OK,
        )
        .unwrap();
    }

    /// Get access to the device config memory.
    fn device_config_memory(&self) -> IoMem;

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

    /// Get notify pointer of a virtqueue. User should send notification (e.g. write 0 to the pointer)
    /// after it add buffers into the corresponding virtqueue.
    fn get_notify_ptr(&self, idx: u16) -> Result<SafePtr<u32, IoMem>, VirtioTransportError>;

    fn is_legacy_version(&self) -> bool;

    // ====================Device interrupt APIs=====================

    /// Register queue interrupt callback. The transport will try to allocate single IRQ line if
    /// `single_interrupt` is set.
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
