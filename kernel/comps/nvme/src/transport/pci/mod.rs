// SPDX-License-Identifier: MPL-2.0

pub(crate) mod device;
pub(crate) mod driver;

use alloc::sync::Arc;

use aster_pci::PCI_BUS;
use spin::Once;

use self::driver::NvmePciDriver;

pub(crate) static NVME_PCI_DRIVER: Once<Arc<NvmePciDriver>> = Once::new();

pub(crate) fn nvme_pci_init() {
    NVME_PCI_DRIVER.call_once(|| Arc::new(NvmePciDriver::new()));
    PCI_BUS
        .lock()
        .register_driver(NVME_PCI_DRIVER.get().unwrap().clone());
}
