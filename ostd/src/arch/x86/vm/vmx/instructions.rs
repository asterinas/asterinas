// SPDX-License-Identifier: MPL-2.0

use crate::{Error, irq::DisabledLocalIrqGuard, prelude::*};

/// Enters VMX operation using the supplied VMXON region.
///
/// # Safety
///
/// 1. The current CPU must support VMX.
/// 2. The current CPU must be outside VMX operation.
/// 3. `CR4.VMXE` must be set, and `CR0` and `CR4` must satisfy the VMX
///    fixed-bit requirements.
/// 4. The `IA32_FEATURE_CONTROL` MSR must be locked and permit VMX operation
///    outside SMX.
/// 5. `vmxon_region` must identify a valid, page-aligned, correctly initialized
///    VMXON region exclusively owned by this CPU. After a successful `VMXON`,
///    the region must not be accessed by software or freed until `VMXOFF` succeeds.
pub(super) unsafe fn vmxon(vmxon_region: Paddr, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
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
/// 1. The current CPU must be in VMX operation.
/// 2. All active VMCSs on the current CPU must have been cleared.
pub(super) unsafe fn vmxoff(_irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
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
/// The current CPU must be in VMX operation.
#[cfg_attr(not(ktest), expect(dead_code))]
pub(super) unsafe fn vmptrst(_irq_guard: &DisabledLocalIrqGuard) -> u64 {
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
