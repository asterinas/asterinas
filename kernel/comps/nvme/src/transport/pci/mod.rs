// SPDX-License-Identifier: MPL-2.0

pub(crate) mod driver;
pub(crate) mod transport;

use alloc::{boxed::Box, collections::VecDeque};

use aster_pci::PCI_BUS;
use ostd::sync::SpinLock;

use self::{driver::NvmePciDriver, transport::NvmePciTransport};

// Transports await functional initialization after PCI driver registration.
static TRANSPORTS: SpinLock<VecDeque<NvmePciTransport>> = SpinLock::new(VecDeque::new());

fn push_device_transport(transport: NvmePciTransport) {
    TRANSPORTS.lock().push_back(transport);
}

pub(crate) fn pop_device_transport() -> Option<NvmePciTransport> {
    TRANSPORTS.lock().pop_front()
}

pub(crate) fn nvme_pci_init() {
    PCI_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(NvmePciDriver))
        .unwrap();
}
