// SPDX-License-Identifier: MPL-2.0

pub mod capability;
pub mod common_cfg;
pub mod device;
pub mod driver;
pub mod legacy;
pub(super) mod msix;

use alloc::sync::Arc;

use ostd::bus::pci::PCI_BUS;
use spin::Once;

use self::driver::VirtioPciDriver;

pub static VIRTIO_PCI_DRIVER: Once<Arc<VirtioPciDriver>> = Once::new();
pub fn virtio_pci_init() {
    VIRTIO_PCI_DRIVER.call_once(|| Arc::new(VirtioPciDriver::new()));
    PCI_BUS
        .lock()
        .register_driver(VIRTIO_PCI_DRIVER.get().unwrap().clone());
}
