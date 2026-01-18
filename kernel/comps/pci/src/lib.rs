// SPDX-License-Identifier: MPL-2.0

//! The PCI bus of Asterinas.
//!
//! Users can implement the bus under the `PciDriver` to register devices to
//! the PCI bus. When the physical device and the driver match successfully, it
//! will be provided through the driver's `construct` function to construct a
//! structure that implements the `PciDevice` trait. And in the end, the PCI
//! bus will store a reference to the structure and finally call the driver's
//! probe function to remind the driver of a new device access.
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

#![no_std]
#![deny(unsafe_code)]

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
mod arch;

pub mod bus;
pub mod capability;
pub mod cfg_space;
pub mod common_device;
mod device_info;

extern crate alloc;

use component::{ComponentInitError, init_component};
pub use device_info::{PciDeviceId, PciDeviceLocation};
use ostd::sync::Mutex;

use self::{bus::PciBus, common_device::PciCommonDevice};

#[init_component]
fn pci_init() -> Result<(), ComponentInitError> {
    init();
    Ok(())
}

/// The PCI bus instance.
pub static PCI_BUS: Mutex<PciBus> = Mutex::new(PciBus::new());

fn init() {
    let Some(all_bus) = arch::init() else {
        log::info!("no PCI bus was found");
        return;
    };
    log::info!("initializing the PCI bus with bus numbers `{:?}`", all_bus);

    let mut lock = PCI_BUS.lock();

    let all_dev = PciDeviceLocation::MIN_DEVICE..=PciDeviceLocation::MAX_DEVICE;
    let all_func = PciDeviceLocation::MIN_FUNCTION..=PciDeviceLocation::MAX_FUNCTION;

    for bus in all_bus {
        for device in all_dev.clone() {
            let mut device_location = PciDeviceLocation {
                bus,
                device,
                function: PciDeviceLocation::MIN_FUNCTION,
            };

            let Some(first_function_device) = PciCommonDevice::new(device_location) else {
                continue;
            };
            let has_multi_function = first_function_device.has_multi_funcs();
            // Register function 0 in advance
            lock.register_common_device(first_function_device);

            if has_multi_function {
                for function in all_func.clone().skip(1) {
                    device_location.function = function;
                    if let Some(common_device) = PciCommonDevice::new(device_location) {
                        lock.register_common_device(common_device);
                    }
                }
            }
        }
    }
}
