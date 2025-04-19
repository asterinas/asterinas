// SPDX-License-Identifier: MPL-2.0

pub mod device;
pub mod driver;

use alloc::sync::Arc;

use ostd::bus::pci::PCI_BUS;
use spin::Once;

use self::driver::NVMePciDriver;

pub static NVME_PCI_DRIVER: Once<Arc<NVMePciDriver>> = Once::new();
pub fn nvme_pci_init() {
    NVME_PCI_DRIVER.call_once(|| Arc::new(NVMePciDriver::new()));
    PCI_BUS
        .lock()
        .register_driver(NVME_PCI_DRIVER.get().unwrap().clone());
}
