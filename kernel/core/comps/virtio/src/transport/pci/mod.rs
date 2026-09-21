// SPDX-License-Identifier: MPL-2.0

mod capability;
mod common_cfg;
mod device;
mod driver;
pub(crate) mod legacy;
pub(super) mod msix;

use alloc::sync::Arc;

use aster_pci::PCI_BUS;
use spin::Once;

use self::driver::VirtioPciDriver;

pub static VIRTIO_PCI_DRIVER: Once<Arc<VirtioPciDriver>> = Once::new();
pub fn virtio_pci_init() {
    VIRTIO_PCI_DRIVER.call_once(|| Arc::new(VirtioPciDriver::new()));
    PCI_BUS
        .lock()
        .register_driver(VIRTIO_PCI_DRIVER.get().unwrap().clone());
}
