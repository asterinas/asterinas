// SPDX-License-Identifier: MPL-2.0

mod device;
mod ioctl;
mod irqfd;
mod vm;

use crate::{device::registry::char, prelude::*};

const KVM_MAJOR: u16 = 10;
const KVM_MINOR: u16 = 232;

pub(super) fn init_in_first_process() -> Result<()> {
    char::register(Arc::new(device::HypervisorDevice))
}
