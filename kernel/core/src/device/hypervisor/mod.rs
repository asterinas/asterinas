// SPDX-License-Identifier: MPL-2.0

//! A minimal KVM device backed by OSTD guest execution and memory management.
//!
//! Each VM owns its memory slots and a single vCPU. Userspace handles port I/O;
//! OSTD owns the VMX, VMCS, and EPT lifetimes.

mod device;
pub(crate) mod ioctl;
mod vcpu_file;
mod vm_file;

use crate::{device::registry::char, prelude::*};

pub(super) fn init_in_first_process() -> Result<()> {
    char::register(Arc::new(device::KvmDevice::default()))
}
