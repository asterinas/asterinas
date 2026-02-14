// SPDX-License-Identifier: MPL-2.0

use self::pci::nvme_pci_init;

pub(crate) mod pci;

pub(crate) fn init() {
    nvme_pci_init();
}
