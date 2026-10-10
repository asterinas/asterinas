// SPDX-License-Identifier: MPL-2.0

//! PCI enumeration, topology, and device-model integration.
//!
//! Drivers implement `aster_device::bus::Driver<PciBus>` and register with [`PCI_BUS`].
//! Devices keep their identity and resources in [`PciCommonDevice`] while bound or unbound.

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

use alloc::{format, sync::Arc};
use core::{fmt, ops::RangeInclusive};

use aster_device::{
    bus::{self as device_bus, BusHandle},
    common::{AnyDevice, BareDevice},
};
use component::{ComponentInitError, init_component};
pub use device_info::{PciDeviceId, PciDeviceLocation};
use spin::Once;

use self::{
    bus::{PciBus, PciDevice},
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
pub static PCI_BUS: Once<Arc<BusHandle<PciBus>>> = Once::new();

fn init() {
    let bus_handle = PCI_BUS.call_once(|| device_bus::register(PciBus).unwrap());
    let Some(all_bus) = arch::init() else {
        ostd::info!("no PCI bus was found");
        return;
    };
    ostd::info!("initializing the PCI bus with bus numbers `{:?}`", all_bus);

    if let Err(err) = enumerate(bus_handle, &all_bus) {
        ostd::error!("cannot enumerate PCI devices: {:?}", err);
    }
}

fn enumerate(bus_handle: &Arc<BusHandle<PciBus>>, range: &RangeInclusive<u8>) -> Result<()> {
    let root = BareDevice::new_root(format!("pci0000:{ROOT_BUS:02x}"));
    aster_device::add_device(&root)?;
    scan_bus(
        bus_handle,
        range,
        ROOT_BUS,
        root,
        &mut [false; MAX_BUSES_PER_SEGMENT],
    )
}

fn scan_bus(
    bus_handle: &Arc<BusHandle<PciBus>>,
    range: &RangeInclusive<u8>,
    bus: u8,
    parent: Arc<dyn AnyDevice>,
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
        register_function(bus_handle, range, common_device, &parent, visited)?;

        if !has_multi_function {
            continue;
        }
        for function in (PciDeviceLocation::MIN_FUNCTION + 1)..=PciDeviceLocation::MAX_FUNCTION {
            location.function = function;
            if let Some(common_device) = PciCommonDevice::new(location) {
                register_function(bus_handle, range, common_device, &parent, visited)?;
            }
        }
    }
    Ok(())
}

fn register_function(
    bus_handle: &Arc<BusHandle<PciBus>>,
    range: &RangeInclusive<u8>,
    common_device: PciCommonDevice,
    parent: &Arc<dyn AnyDevice>,
    visited: &mut [bool; MAX_BUSES_PER_SEGMENT],
) -> Result<()> {
    let device_type = common_device.device_type();
    let location = *common_device.location();
    let name = format!(
        "0000:{:02x}:{:02x}.{}",
        location.bus, location.device, location.function
    );
    let device = PciDevice::builder(bus_handle, name, common_device)
        .parent(parent.clone())
        .build();
    aster_device::add_device(&device)?;

    const UNCONFIGURED_SECONDARY_BUS: u8 = 0;
    if let PciDeviceType::PciToPciBridge(_, secondary, _) = device_type
        && secondary != UNCONFIGURED_SECONDARY_BUS
    {
        // An invalid bridge edge must not stop enumeration of unrelated devices.
        match scan_bus(bus_handle, range, secondary, device, visited) {
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
    /// The device model failed to register a PCI function or root device.
    DeviceModel(aster_device::common::Error),
}

type Result<T, E = Error> = core::result::Result<T, E>;

impl From<aster_device::common::Error> for Error {
    fn from(error: aster_device::common::Error) -> Self {
        Self::DeviceModel(error)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTopology => f.write_str("InvalidTopology"),
            Self::DeviceModel(error) => f.debug_tuple("DeviceModel").field(error).finish(),
        }
    }
}
