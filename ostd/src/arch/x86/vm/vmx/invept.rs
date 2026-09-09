// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use x86::msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr};

use super::{VmxGuard, instructions};
use crate::{Error, cpu::CpuSet, irq, prelude::*, smp, sync::SpinLock};

static EPT_OPERATION_LOCK: SpinLock<()> = SpinLock::new(());
static EPT_OPERATION_FAILED: AtomicBool = AtomicBool::new(false);

/// Keeps VMX and the capabilities required for EPT invalidation available.
pub(crate) struct EptGuard {
    _vmx_guard: VmxGuard,
}

impl EptGuard {
    pub(crate) fn new() -> Result<Self> {
        let guard = Self {
            _vmx_guard: VmxGuard::acquire_vmx()?,
        };
        if !guard.run_on_all_cpus(Self::check_support_on_cpu) {
            return Err(Error::NotEnoughResources);
        }
        Ok(guard)
    }

    /// Invalidates EPT-derived translations on every CPU before returning.
    ///
    /// # Panics
    ///
    /// Panics if local IRQs are disabled.
    pub(crate) fn invalidate(&self) -> Result<()> {
        if !self.run_on_all_cpus(Self::invalidate_on_cpu) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    fn run_on_all_cpus(&self, call_fn: fn()) -> bool {
        assert!(crate::arch::irq::is_local_enabled());
        // Serialize requests that share the result flag. This lock keeps IRQs
        // enabled so another CPU can complete an IPI while waiting for it.
        let _lock = EPT_OPERATION_LOCK.lock();
        EPT_OPERATION_FAILED.store(false, Ordering::Relaxed);
        smp::inter_processor_call(&CpuSet::new_full(), call_fn).wait();
        !EPT_OPERATION_FAILED.load(Ordering::Relaxed)
    }

    fn check_support_on_cpu() {
        let _irq_guard = irq::disable_local();
        // SAFETY: The live VmxGuard checked VMX support on every CPU.
        let primary = unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS) };
        if primary & (1 << 63) == 0 {
            EPT_OPERATION_FAILED.store(true, Ordering::Relaxed);
            return;
        }
        // SAFETY: The primary controls above advertise secondary controls.
        let secondary = unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS2) };
        if secondary & (1 << 33) == 0 {
            EPT_OPERATION_FAILED.store(true, Ordering::Relaxed);
            return;
        }
        // SAFETY: This CPU advertises EPT in its secondary controls above.
        let cap = unsafe { rdmsr(IA32_VMX_EPT_VPID_CAP) };
        // Intel SDM, Vol. 3D, Appendix A.10: four-level walks, WB page tables,
        // INVEPT, and all-context invalidation are required by this backend.
        const REQUIRED: u64 = (1 << 6) | (1 << 14) | (1 << 20) | (1 << 26);
        if cap & REQUIRED != REQUIRED {
            EPT_OPERATION_FAILED.store(true, Ordering::Relaxed);
        }
    }

    fn invalidate_on_cpu() {
        let irq_guard = irq::disable_local();
        // SAFETY:
        // 1. This callback is dispatched with a live EptGuard, which keeps
        //    every CPU in VMX operation until all callbacks have completed.
        // 2. EptGuard::new checked all-context INVEPT support on every CPU.
        if unsafe { instructions::invept_all_contexts(&irq_guard) }.is_err() {
            EPT_OPERATION_FAILED.store(true, Ordering::Relaxed);
        }
    }
}
