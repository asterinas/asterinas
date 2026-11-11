// SPDX-License-Identifier: MPL-2.0

pub(in crate::kvm) struct VmArch {}

impl VmArch {
    pub(in crate::kvm)  fn new() -> Self {
        Self {}
    }

    pub(in crate::kvm)  fn init(&self){
        ostd::arch::vm::enable_virtualization_cpu();
    }
}

impl Drop for VmArch {
    fn drop(&mut self) {
        ostd::arch::vm::disable_virtualization_cpu()
    }
}
