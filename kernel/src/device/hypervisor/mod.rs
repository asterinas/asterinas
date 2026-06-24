mod apic;
mod device;
mod ioctl;
mod vcpu;
mod vcpu_file;
mod vm;
mod vm_file;

pub use device::HypervisorDevice;

use crate::{device::registry::char, fs::vfs::path::PathResolver, prelude::*};

const KVM_MAJOR: u16 = 10;
const KVM_MINOR: u16 = 232;

pub(super) fn init_in_first_process(_path_resolver: &PathResolver) -> Result<()> {
    ostd::vm::init()?;
    char::register(Arc::new(HypervisorDevice::new()))?;
    Ok(())
}
