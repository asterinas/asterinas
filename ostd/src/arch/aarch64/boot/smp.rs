// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support

use crate::{boot::smp::PerApRawInfo, mm::Paddr};

pub(crate) fn count_processors() -> Option<u32> {
    // TODO: Parse CPU count from device tree
    Some(1)
}

/// Brings up all application processors.
///
/// # Safety
///
/// The caller must ensure that
///  1. we're in the boot context of the BSP,
///  2. all APs have not yet been booted, and
///  3. the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(
    _info_ptr: *const PerApRawInfo,
    _pt_ptr: Paddr,
    num_cpus: u32,
) {
    if num_cpus <= 1 {
        return;
    }

    crate::info!("Bootstrapping CPU is 0, booting all other CPUs");

    // TODO: Implement PSCI-based AP boot
    crate::warn!("SMP boot not yet implemented for aarch64");
}
