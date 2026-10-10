// SPDX-License-Identifier: MPL-2.0

mod capability;
mod common_cfg;
mod device;
mod driver;
pub(crate) mod legacy;
pub(super) mod msix;

use alloc::{boxed::Box, collections::VecDeque};

use aster_pci::PCI_BUS;
use ostd::sync::SpinLock;

use self::driver::VirtioPciDriver;
use crate::transport::VirtioTransport;

static TRANSPORTS: SpinLock<VecDeque<Box<dyn VirtioTransport>>> = SpinLock::new(VecDeque::new());

fn push_device_transport(transport: Box<dyn VirtioTransport>) {
    TRANSPORTS.lock().push_back(transport);
}

pub(crate) fn pop_device_transport() -> Option<Box<dyn VirtioTransport>> {
    TRANSPORTS.lock().pop_front()
}

pub(super) fn virtio_pci_init() {
    PCI_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(VirtioPciDriver))
        .unwrap();
}
