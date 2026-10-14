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

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "pci: "
    };
}

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
#[cfg_attr(target_arch = "aarch64", path = "arch/arm/mod.rs")]
mod arch;

pub mod bus;
pub mod capability;
pub mod cfg_space;
pub mod common_device;
mod device_info;

extern crate alloc;

use core::{fmt, ops::RangeInclusive};

use component::{ComponentInitError, init_component};
pub use device_info::{PciDeviceId, PciDeviceLocation};
use ostd::sync::Mutex;

use self::{
    bus::PciBus,
    common_device::{PciCommonDevice, PciDeviceType},
};

// FIXME: Discover root buses from firmware instead of assuming bus 0 is the only root.
// The accessible bus range does not identify root buses.
// Until firmware root discovery is supported, use bus 0 as the root of the device tree,
// matching our QEMU Q35 configuration with a single root bus (`pcie.0`).
// Reference: <https://github.com/qemu/qemu/blob/master/docs/pcie.txt>.
const ROOT_BUS: u8 = 0;
const MAX_BUSES_PER_SEGMENT: usize = 256;

#[init_component]
fn pci_init() -> Result<(), ComponentInitError> {
    init();
    Ok(())
}

/// The PCI bus instance.
pub static PCI_BUS: Mutex<PciBus> = Mutex::new(PciBus::new());

fn init() {
    let Some(all_bus) = arch::init() else {
        ostd::info!("no PCI bus was found");
        return;
    };
    ostd::info!("initializing the PCI bus with bus numbers `{:?}`", all_bus);

    if let Err(err) = scan_bus(
        &mut PCI_BUS.lock(),
        &all_bus,
        ROOT_BUS,
        &mut [false; MAX_BUSES_PER_SEGMENT],
    ) {
        ostd::error!("cannot enumerate PCI devices: {:?}", err);
    }
}

fn scan_bus(
    pci_bus: &mut PciBus,
    range: &RangeInclusive<u8>,
    bus: u8,
    visited: &mut [bool; MAX_BUSES_PER_SEGMENT],
) -> Result<()> {
    if !range.contains(&bus) {
        return Err(Error::InvalidTopology);
    }

    // A bus may be reachable through multiple bridges, as on the i450NX chipset.
    // Skip buses already scanned to avoid duplicate devices and recursion cycles.
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/pci/probe.c#L1400-L1405>.
    if visited[bus as usize] {
        return Ok(());
    }
    visited[bus as usize] = true;

    for device in PciDeviceLocation::MIN_DEVICE..=PciDeviceLocation::MAX_DEVICE {
        let mut location = PciDeviceLocation {
            bus,
            device,
            function: PciDeviceLocation::MIN_FUNCTION,
        };
        let Some(common_device) = PciCommonDevice::new(location) else {
            continue;
        };
        let has_multi_function = common_device.has_multi_funcs();
        register_function(pci_bus, range, common_device, visited)?;

        if !has_multi_function {
            continue;
        }
        for function in (PciDeviceLocation::MIN_FUNCTION + 1)..=PciDeviceLocation::MAX_FUNCTION {
            location.function = function;
            if let Some(common_device) = PciCommonDevice::new(location) {
                register_function(pci_bus, range, common_device, visited)?;
            }
        }
    }
    Ok(())
}

fn register_function(
    pci_bus: &mut PciBus,
    range: &RangeInclusive<u8>,
    common_device: PciCommonDevice,
    visited: &mut [bool; MAX_BUSES_PER_SEGMENT],
) -> Result<()> {
    let device_type = common_device.device_type();
    let location = *common_device.location();
    pci_bus.register_common_device(common_device);

    const UNCONFIGURED_SECONDARY_BUS: u8 = 0;
    if let PciDeviceType::PciToPciBridge(_, secondary, _) = device_type
        && secondary != UNCONFIGURED_SECONDARY_BUS
    {
        // An invalid bridge edge must not stop enumeration of unrelated devices.
        match scan_bus(pci_bus, range, secondary, visited) {
            Err(Error::InvalidTopology) => ostd::warn!(
                "skipping bridge {:02x}:{:02x}.{} secondary bus {:02x}: outside accessible range",
                location.bus,
                location.device,
                location.function,
                secondary
            ),
            result => result?,
        }
    }
    Ok(())
}

enum Error {
    /// A bus number is outside the accessible range.
    InvalidTopology,
}

type Result<T, E = Error> = core::result::Result<T, E>;

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTopology => f.write_str("InvalidTopology"),
        }
    }
}
