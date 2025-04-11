// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use self::pci::nvme_pci_init;

pub mod pci;

#[derive(Debug, PartialEq, Eq)]
pub enum NVMeTransportError {
    DeviceStatusError,
    InvalidArgs,
}

pub fn init() {
    nvme_pci_init();
}
