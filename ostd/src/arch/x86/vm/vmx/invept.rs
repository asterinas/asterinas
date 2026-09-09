// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use x86::msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr};

use super::{VmxGuard, instructions};
use crate::{Error, cpu::CpuSet, irq, prelude::*, smp, sync::SpinLock};

static EPT_OPERATION_LOCK: SpinLock<()> = SpinLock::new(());
static EPT_OPERATION_FAILED: AtomicBool = AtomicBool::new(false);
pub(crate) static SUPPORT_EPT: AtomicBool = AtomicBool::new(false);

pub(crate) fn check_invept_support() {
    let _irq_guard = irq::disable_local();
    let primary = unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS) };
    if primary & (1 << 63) == 0 {
        return;
    }
    let secondary = unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS2) };
    if secondary & (1 << 33) == 0 {
        return;
    }
    let cap = unsafe { rdmsr(IA32_VMX_EPT_VPID_CAP) };
    // Intel SDM, Vol. 3D, Appendix A.10: four-level walks, WB page tables,
    // INVEPT, and all-context invalidation are required by this backend.
    const REQUIRED: u64 = (1 << 6) | (1 << 14) | (1 << 20) | (1 << 26);
    if cap & REQUIRED != REQUIRED {
        return;
    }
    SUPPORT_EPT.store(true, Ordering::Relaxed);
}

/// Invalidates EPT-derived translations on every CPU before returning.
///
/// # Panics
///
/// Panics if `SUPPORT_EPT` is false or local IRQs are disabled.
pub(crate) fn invalidate(_vmx_guard: &VmxGuard) -> Result<()> {
    assert!(SUPPORT_EPT.load(Ordering::Relaxed));
    assert!(crate::arch::irq::is_local_enabled());

    // Serialize requests that share the result flag.
    // IRQs are enabled so another CPU can complete an IPI while waiting for it.
    let _lock = EPT_OPERATION_LOCK.lock();
    EPT_OPERATION_FAILED.store(false, Ordering::Relaxed);
    smp::inter_processor_call(&CpuSet::new_full(), || {
        let irq_guard = irq::disable_local();
        // SAFETY:
        // 1. `_vmx_guard` ensures that the CPU remains in VMX operation.
        // 2. `SUPPORT_EPT` has been checked before invoking this callback.
        if unsafe { instructions::invept_all_contexts(&irq_guard) }.is_err() {
            EPT_OPERATION_FAILED.store(true, Ordering::Relaxed);
        }
    })
    .wait();

    if EPT_OPERATION_FAILED.load(Ordering::Relaxed) {
        return Err(Error::AccessDenied);
    }
    Ok(())
}
