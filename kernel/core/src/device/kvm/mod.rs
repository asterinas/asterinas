// SPDX-License-Identifier: MPL-2.0

//! A minimal KVM device backed by OSTD guest execution and memory management.
//!
//! Each VM owns its memory slots and a single vCPU. Userspace handles port I/O;
//! OSTD owns the VMX, VMCS, and EPT lifetimes.

mod device;
pub(crate) mod ioctl;
mod vcpu_file;
mod vm_file;

use ostd::arch::cpu::extension::{IsaExtensions, has_extensions};

use crate::{device::registry::char, prelude::*};

pub(super) fn init_in_first_process() -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    if !has_extensions(IsaExtensions::VMX) {
        info!("Skipping KVM device registration: VMX is unavailable");
        return Ok(());
    }

    char::register(Arc::new(device::KvmDevice::default()))
}
