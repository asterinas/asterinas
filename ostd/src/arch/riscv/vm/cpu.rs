// SPDX-License-Identifier: MPL-2.0

use super::csr::{Hcounteren, Hedeleg, Hideleg, Hvip, Vsie};

/// Enables the virtualization for cpu.
///
/// This function should be registered with the `cpu_online` initialization routine
/// so that it is called and managed automatically.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/virt/kvm/kvm_main.c#L5609>.
///
/// TODO: For now, place this within the call to `vm::init`;
/// it will be migrated once the relevant mechanisms in Asterinas are further refined.
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

/// Disables the virtualization for cpu.
///
/// This function should be registered with the `cpu_offline` initialization routine
/// so that it is called and managed automatically.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/virt/kvm/kvm_main.c#L5629>.
///
/// TODO: For now, place this within the call to `vm::drop`;
/// it will be migrated once the relevant mechanisms in Asterinas are further refined.
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
