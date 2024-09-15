// SPDX-License-Identifier: MPL-2.0

//! MMIO bus.

#![allow(unused_variables)]

use alloc::{collections::VecDeque, fmt::Debug, sync::Arc, vec::Vec};

use log::{debug, error};

use super::common_device::MmioCommonDevice;
use crate::bus::BusProbeError;

/// MMIO device trait
pub trait MmioDevice: Sync + Send + Debug {
    /// Device ID
    fn device_id(&self) -> u32;
}

/// MMIO device driver.
pub trait MmioDriver: Sync + Send + Debug {
    /// Probe an unclaimed mmio device.
    ///
    /// If the driver matches and succeeds in initializing the unclaimed device,
    /// then the driver will return an claimed instance of the device,
    /// signaling that the mmio device is now ready to work.
    ///
    /// Once a device is matched and claimed by a driver,
    /// it won't be fed to another driver for probing.
    fn probe(
        &self,
        device: MmioCommonDevice,
    ) -> Result<Arc<dyn MmioDevice>, (BusProbeError, MmioCommonDevice)>;
}

/// MMIO bus
pub struct MmioBus {
    common_devices: VecDeque<MmioCommonDevice>,
    devices: Vec<Arc<dyn MmioDevice>>,
    drivers: Vec<Arc<dyn MmioDriver>>,
}

impl MmioBus {
    /// Registers a MMIO driver to the MMIO bus.
    pub fn register_driver(&mut self, driver: Arc<dyn MmioDriver>) {
        debug!("Register driver:{:#x?}", driver);
        let length = self.common_devices.len();
        for i in (0..length).rev() {
            let common_device = self.common_devices.pop_front().unwrap();
            let device_id = common_device.read_device_id().unwrap();
            let device = match driver.probe(common_device) {
                Ok(device) => {
                    debug_assert!(device_id == device.device_id());
                    self.devices.push(device);
                    continue;
                }
                Err((err, device)) => {
                    if err != BusProbeError::DeviceNotMatch {
                        error!("MMIO device construction failed, reason: {:?}", err);
                    }
                    device
                }
            };
            self.common_devices.push_back(device);
        }
        self.drivers.push(driver);
    }

    pub(super) fn register_mmio_device(&mut self, mut mmio_device: MmioCommonDevice) {
        let device_id = mmio_device.read_device_id().unwrap();
        for driver in self.drivers.iter() {
            mmio_device = match driver.probe(mmio_device) {
                Ok(device) => {
                    debug_assert!(device_id == device.device_id());
                    self.devices.push(device);
                    return;
                }
                Err((err, common_device)) => {
                    if err != BusProbeError::DeviceNotMatch {
                        error!("MMIO device construction failed, reason: {:?}", err);
                    }
                    common_device
                }
            };
        }
        self.common_devices.push_back(mmio_device);
    }

    pub(super) const fn new() -> Self {
        Self {
            common_devices: VecDeque::new(),
            devices: Vec::new(),
            drivers: Vec::new(),
        }
    }
}
