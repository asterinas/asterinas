// SPDX-License-Identifier: MPL-2.0

pub(crate) mod driver;
pub(crate) mod transport;

use alloc::{boxed::Box, vec::Vec};

use aster_pci::PCI_BUS;
use ostd::sync::SpinLock;

use self::{driver::NvmePciDriver, transport::NvmePciTransport};

// Transports await functional initialization after PCI driver registration.
static TRANSPORTS: SpinLock<Vec<NvmePciTransport>> = SpinLock::new(Vec::new());

fn push_device_transport(transport: NvmePciTransport) {
    TRANSPORTS.lock().push(transport);
}

pub(crate) fn pop_device_transport() -> Option<NvmePciTransport> {
    TRANSPORTS.lock().pop()
}

pub(crate) fn nvme_pci_init() {
    PCI_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(NvmePciDriver))
        .unwrap();
}
