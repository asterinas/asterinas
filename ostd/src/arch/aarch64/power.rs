// SPDX-License-Identifier: MPL-2.0

//! Power management via the PSCI firmware interface.
//!
//! TODO: Read the PSCI conduit (`hvc`/`smc`) from the device tree instead of
//! assuming `hvc`, which is the QEMU `virt` default when EL2 is present.

use crate::power::{ExitCode, inject_poweroff_handler, inject_restart_handler};

const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

fn psci_call(function: u64) {
    // SAFETY: Issuing a PSCI call has no memory-safety implications; on success
    // it does not return.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            in("x0") function,
            options(nostack, nomem),
        );
    }
}

fn try_poweroff(_code: ExitCode) {
    psci_call(PSCI_SYSTEM_OFF);
}

fn try_restart(_code: ExitCode) {
    psci_call(PSCI_SYSTEM_RESET);
}

pub(super) fn init() {
    inject_poweroff_handler(try_poweroff);
    inject_restart_handler(try_restart);
}
