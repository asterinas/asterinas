// SPDX-License-Identifier: MPL-2.0

use super::csr::{Hcounteren, Hedeleg, Hideleg, Hvip, Vsie};

/// Enable the virtualization for cpu.
/// This function should be registered with the `cpu_online` initialization routine
/// so that it is called and managed automatically.
///
/// For further details, refer to the Linux implementation.
/// About function: static int kvm_online_cpu(unsigned int cpu) {}
/// https://github.com/torvalds/linux/blob/master/virt/kvm/kvm_main.c
pub fn enable_virtualization_cpu() {
    // SAFETY: The timing of the invocation is determined by the caller,
    // and this configuration is applied before any actual KVM operations are executed.
    unsafe {
        Hedeleg::write(0);
        Hideleg::write(0);

        // VS should access only the time counter directly.
        // Everything else should trap.
        Hcounteren::write(0x02);

        Hvip::write(0);
    }
}

/// Disable the virtualization for cpu.
/// This function should be registered with the `cpu_offline` initialization routine
/// so that it is called and managed automatically.
///
/// For further details, refer to the Linux implementation.
/// About function: static int kvm_offline_cpu(unsigned int cpu) {}
/// https://github.com/torvalds/linux/blob/master/virt/kvm/kvm_main.c
pub fn disable_virtualization_cpu() {
    // SAFETY: The timing of the invocation is determined by the caller,
    // and the clearing of this configuration serves as the final step before all virtualization features are disabled.
    unsafe {
        Vsie::write(0);
        Hvip::write(0);
        Hedeleg::write(0);
        Hideleg::write(0);
    }
}
