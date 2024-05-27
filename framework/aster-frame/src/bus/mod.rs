// SPDX-License-Identifier: MPL-2.0

pub mod mmio;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BusProbeError {
    DeviceNotMatch,
    ConfigurationSpaceError,
}

pub fn init() {
    mmio::init();
}
