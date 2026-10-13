// SPDX-License-Identifier: MPL-2.0

//! KVM device (kvm) support.
//!
//! KVM device with major number 10.
//! Devices appear as `/dev/kvm`

mod device;
mod vcpu_file;
mod vm_file;
mod ioctl;

use device::KvmDevice;

use crate::{device::registry::char, prelude::*};

const KVM_MAJOR: u16 = 10;
const KVM_MINOR: u16 = 232;

pub(super) fn init_in_first_process() -> Result<()> {
    char::register(Arc::new(KvmDevice::new()))?;
    Ok(())
}
