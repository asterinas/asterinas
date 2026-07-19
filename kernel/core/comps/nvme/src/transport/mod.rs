// SPDX-License-Identifier: MPL-2.0

pub(crate) mod pci;

pub(crate) fn init() {
    pci::nvme_pci_init();
}
