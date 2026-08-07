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

    // SAFETY: The caller upholds the architectural requirements documented by
    // this function. The memory operand contains the physical address of the
    // VMXON region and remains valid for the duration of the instruction.
    unsafe {
        core::arch::asm!(
            "vmxon [{region}]",
            "setna {failed}",
            region = in(reg) &vmxon_region,
            failed = lateout(reg_byte) failed,
            options(nostack, readonly)
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

    // SAFETY: The caller guarantees that the current CPU can execute
    // `VMXOFF`. The instruction has no memory operand.
    unsafe {
        core::arch::asm!(
            "vmxoff",
            "setna {failed}",
            failed = lateout(reg_byte) failed,
            options(nomem, nostack)
        );
    }

    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}
