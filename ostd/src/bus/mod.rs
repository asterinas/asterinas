// SPDX-License-Identifier: MPL-2.0

//! Bus operations

// TODO: Extract the bus operations into a separate module.

/// An error that occurs during bus probing.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BusProbeError {
    /// The device does not match the expected criteria.
    DeviceNotMatch,
    /// An error in accessing the configuration space of the device.
    ConfigurationSpaceError,
}
