// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aster_pci::cfg_space::BarAccess;
use aster_util::safe_ptr::SafePtr;
use ostd::{
    arch::device::io_port::{PortRead, PortWrite},
    io::IoMem,
    irq::IrqCallbackFunction,
    mm::{PodOnce, dma::DmaCoherent},
};
use ostd_pod::Pod;

use self::{mmio::virtio_mmio_init, pci::virtio_pci_init};
use crate::{
    VirtioDeviceType,
    queue::{AvailRing, Descriptor, UsedRing},
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

    /// Get access to the device config memory.
    fn device_config_mem(&self) -> Option<IoMem>;

    /// Get access to the device config BAR space.
    fn device_config_bar(&self) -> Option<(BarAccess, usize)>;

    // ====================Virtqueue related APIs====================

    /// Get the total number of queues
    fn num_queues(&self) -> u16;

    /// Set virtqueue information. Some transport may set other necessary information such as MSI-X vector in PCI transport.
    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, Arc<DmaCoherent>>,
        avail_ring_ptr: &SafePtr<AvailRing, Arc<DmaCoherent>>,
        used_ring_ptr: &SafePtr<UsedRing, Arc<DmaCoherent>>,
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
    legacy_space: Option<(BarAccess, usize)>,
}

impl<T: Pod> ConfigManager<T> {
    pub(super) fn new(
        modern_space: Option<SafePtr<T, IoMem>>,
        legacy_space: Option<(BarAccess, usize)>,
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

#[derive(Debug, Eq, PartialEq)]
pub enum VirtioTransportError {
    DeviceStatusError,
    InvalidArgs,
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

/// A wrapper around [`Box<dyn VirtioTransport>`] that sets the device status to FAILED
/// on drop unless [`Self::finish_init`] has been called.
///
/// This ensures that the FAILED status bit is set whenever a device initialization
/// is aborted before completion, as required by the VirtIO specification.
///
/// The `DeviceTransport` is stored in the device struct and held for the device's
/// lifetime. During initialization it acts as a sentinel: if init fails (the struct
/// is dropped without [`finish_init`] being called), [`Drop`] sets `FAILED`. Once
/// `finish_init` is called, the FAILED bit is no longer set on drop.
#[derive(Debug)]
pub struct DeviceTransport {
    inner: Box<dyn VirtioTransport>,
    is_completed: bool,
}

impl DeviceTransport {
    /// Creates a new wrapper around `transport`.
    ///
    /// The wrapper will set the device status to [`DeviceStatus::FAILED`] on drop
    /// unless [`Self::finish_init`] is called first.
    pub fn new(transport: Box<dyn VirtioTransport>) -> Self {
        Self {
            inner: transport,
            is_completed: false,
        }
    }

    /// Marks initialization as complete and sets the device status to `DRIVER_OK`.
    ///
    /// After this call, the wrapper will NOT set `FAILED` on drop.
    pub fn finish_init(&mut self) {
        self.is_completed = true;
        let status = self.inner.read_device_status() | DeviceStatus::DRIVER_OK;
        self.inner.write_device_status(status).unwrap();
    }
}

impl Deref for DeviceTransport {
    type Target = Box<dyn VirtioTransport>;

    fn deref(&self) -> &Box<dyn VirtioTransport> {
        &self.inner
    }
}

impl DerefMut for DeviceTransport {
    fn deref_mut(&mut self) -> &mut Box<dyn VirtioTransport> {
        &mut self.inner
    }
}

impl Drop for DeviceTransport {
    fn drop(&mut self) {
        if !self.is_completed {
            let status = self.inner.read_device_status();
            let _ = self
                .inner
                .write_device_status(status | DeviceStatus::FAILED);
        }
    }
}

pub fn init() {
    virtio_pci_init();
    virtio_mmio_init();
}
