// SPDX-License-Identifier: MPL-2.0

mod capability;
mod common_cfg;
mod device;
mod driver;
pub(crate) mod legacy;
pub(super) mod msix;

use alloc::boxed::Box;

use aster_pci::PCI_BUS;

use self::driver::VirtioPciDriver;

pub(super) fn virtio_pci_init() {
    PCI_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(VirtioPciDriver))
        .unwrap();
}
