// SPDX-License-Identifier: MPL-2.0

//! PCI bus
//!
//! Users can implement the bus under the `PciDriver` to the PCI bus to register devices,
//! when the physical device and the driver match successfully, it will be provided through the driver `construct` function
//! to construct a structure that implements the `PciDevice` trait. And in the end,
//! PCI bus will store a reference to the structure and finally call the driver's probe function to remind the driver of a new device access.
//!
//! Use case:
//!
//! ```rust no_run
//! #[derive(Debug)]
//! pub struct PciDeviceA {
//!     common_device: PciCommonDevice,
//! }
//!
//! impl PciDevice for PciDeviceA {
//!     fn device_id(&self) -> PciDeviceId {
//!         self.common_device.device_id().clone()
//!     }
//! }
//!
//! #[derive(Debug)]
//! pub struct PciDriverA {
//!     devices: Mutex<Vec<Arc<PciDeviceA>>>,
//! }
//!
//! impl PciDriver for PciDriverA {
//!     fn probe(
//!         &self,
//!         device: PciCommonDevice,
//!     ) -> Result<Arc<dyn PciDevice>, (PciDriverProbeError, PciCommonDevice)> {
//!         if device.device_id().vendor_id != 0x1234 {
//!             return Err((PciDriverProbeError::DeviceNotMatch, device));
//!         }
//!         let device = Arc::new(PciDeviceA {
//!             common_device: device,
//!         });
//!         self.devices.lock().push(device.clone());
//!         Ok(device)
//!     }
//! }
//!
//! pub fn driver_a_init() {
//!     let driver_a = Arc::new(PciDriverA {
//!         devices: Mutex::new(Vec::new()),
//!     });
//!     PCI_BUS.lock().register_driver(driver_a);
//! }
//! ```

pub mod bus;
pub mod capability;
pub mod cfg_space;
pub mod common_device;
mod device_info;

pub use device_info::{PciDeviceId, PciDeviceLocation};

use self::{bus::PciBus, common_device::PciCommonDevice};
use crate::sync::Mutex;

/// PCI bus instance
pub static PCI_BUS: Mutex<PciBus> = Mutex::new(PciBus::new());

pub(crate) fn init() {
    let mut lock = PCI_BUS.lock();
    for location in PciDeviceLocation::all() {
        let Some(device) = PciCommonDevice::new(location) else {
            continue;
        };
        lock.register_common_device(device);
    }
}
