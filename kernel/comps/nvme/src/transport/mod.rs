// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use self::pci::nvme_pci_init;

pub(crate) mod pci;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NVMeTransportError {
    DeviceStatusError,
    InvalidArgs,
}

pub(crate) fn init() {
    nvme_pci_init();
}
