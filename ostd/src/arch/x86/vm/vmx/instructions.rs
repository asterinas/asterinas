// SPDX-License-Identifier: MPL-2.0

use crate::{Error, prelude::*};

/// Enters VMX operation using the supplied VMXON region.
///
/// # Safety
///
/// The caller must ensure that the current CPU meets all `VMXON` prerequisites
/// and that `vmxon_region` identifies a correctly initialized, exclusively
/// owned VMXON region that remains alive until `VMXOFF` completes.
pub(super) unsafe fn vmxon(vmxon_region: Paddr) -> Result<()> {
    let failed: u8;

    // SAFETY: The caller ensures safety.
    unsafe {
        core::arch::asm!(
            "vmxon [{region}]",
            "setna {failed}",
            region = in(reg) &vmxon_region,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }

    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

/// Leaves VMX operation on the current CPU.
///
/// # Safety
///
/// The caller must ensure that the current CPU is in VMX root operation and
/// that it has no active VMCS requiring cleanup.
pub(super) unsafe fn vmxoff() -> Result<()> {
    let failed: u8;

    // SAFETY: The caller ensures safety.
    unsafe {
        core::arch::asm!(
            "vmxoff",
            "setna {failed}",
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }

    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

/// Returns the current VMCS pointer.
///
/// A logical processor with no current VMCS returns `u64::MAX`.
///
/// # Safety
///
/// The caller must ensure that the current CPU is in VMX root operation.
#[cfg_attr(not(ktest), expect(dead_code))]
pub(super) unsafe fn vmptrst() -> u64 {
    let mut current_vmcs = 0_u64;

    // SAFETY: The caller ensures safety.
    unsafe {
        core::arch::asm!(
            "vmptrst [{current_vmcs_ptr}]",
            current_vmcs_ptr = in(reg) &mut current_vmcs,
            options(nostack)
        );
    }

    current_vmcs
}
