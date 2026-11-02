// SPDX-License-Identifier: MPL-2.0

mod capability;
mod common_cfg;
mod device;
mod driver;
pub(crate) mod legacy;
pub(super) mod msix;

use alloc::{boxed::Box, collections::VecDeque, sync::Arc};

use aster_pci::PCI_BUS;
use ostd::sync::SpinLock;

use self::driver::VirtioPciDriver;
use crate::bus::VirtioDevice;

static DEVICES: SpinLock<VecDeque<Arc<VirtioDevice>>> = SpinLock::new(VecDeque::new());

fn push_device(device: Arc<VirtioDevice>) {
    DEVICES.lock().push_back(device);
}

pub(crate) fn pop_device() -> Option<Arc<VirtioDevice>> {
    DEVICES.lock().pop_front()
}

pub(super) fn virtio_pci_init() {
    PCI_BUS
        .get()
        .unwrap()
        .register_driver(Box::new(VirtioPciDriver))
        .unwrap();
}
