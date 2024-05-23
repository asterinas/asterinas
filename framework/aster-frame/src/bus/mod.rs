// SPDX-License-Identifier: MPL-2.0

//! Bus operations

// FIXME: remove this lint when the docs of the whole bus module are added.
#![allow(missing_docs)]

pub mod mmio;
pub mod pci;

/// An error that occurs during bus probing.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BusProbeError {
    /// The device does not match the expected criteria.
    DeviceNotMatch,
    /// An error in accessing the configuration space of the device.
    ConfigurationSpaceError,
}

/// Initializes the bus
pub fn init() {
    pci::init();
    mmio::init();
}
