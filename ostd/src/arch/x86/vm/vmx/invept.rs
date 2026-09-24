// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use x86::{
    msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr},
    vmx::vmcs::control::{PrimaryControls, SecondaryControls},
};

use super::{VmxGuard, instructions};
use crate::{
    cpu::CpuSet,
    cpu_local, irq,
    mm::{Frame, frame::meta::AnyFrameMeta},
    prelude::*,
    smp::{self, PendingIpis},
    sync::{LocalIrqDisabled, RcuDrop, SpinLock},
};

static HAS_EPT_UNSUPPORTED_CPU: AtomicBool = AtomicBool::new(false);
static IS_EPT_SUPPORTED: AtomicBool = AtomicBool::new(false);

struct PendingInvalidation {
    // Permission changes require invalidation even without frames to release.
    pending: bool,
    // Released only after INVEPT and an RCU grace period.
    frames: Vec<Frame<dyn AnyFrameMeta>>,
}

cpu_local! {
    static PENDING_INVALIDATION: SpinLock<PendingInvalidation, LocalIrqDisabled> =
        SpinLock::new(PendingInvalidation {
            pending: false,
            frames: Vec::new(),
        });
}

pub(in crate::arch::vm) fn init_ept_support() {
    if !check_ept_support() {
        HAS_EPT_UNSUPPORTED_CPU.store(true, Ordering::Relaxed);
    }
}

fn check_ept_support() -> bool {
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

    if !has_extensions(IsaExtensions::VMX) {
        return false;
    }

    // The upper 32 bits of each control MSR specify which controls may be set to 1.
    let primary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS) } >> 32) as u32;
    if primary_allowed_1 & PrimaryControls::SECONDARY_CONTROLS.bits() == 0 {
        return false;
    }
    let secondary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS2) } >> 32) as u32;
    if secondary_allowed_1 & SecondaryControls::ENABLE_EPT.bits() == 0 {
        return false;
    }

    let cap = unsafe { rdmsr(IA32_VMX_EPT_VPID_CAP) };
    // Intel SDM, Vol. 3D, Appendix A.10.
    const REQUIRED: u64 =
        // Four-level EPT page walks.
        (1 << 6)
        // Write-back memory type for EPT paging structures.
        | (1 << 14)
        // INVEPT instruction.
        | (1 << 20)
        // All-context INVEPT.
        | (1 << 26);
    if cap & REQUIRED != REQUIRED {
        return false;
    }
    true
}

/// Initializes global EPT support.
///
/// # Safety
///
/// This function must be called after `init_ept_support` has been called on
/// each CPU, including the bootstrap processor and the application processors.
pub(in crate::arch) unsafe fn init() {
    IS_EPT_SUPPORTED.store(
        !HAS_EPT_UNSUPPORTED_CPU.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

pub(crate) fn is_ept_supported() -> bool {
    IS_EPT_SUPPORTED.load(Ordering::Relaxed)
}

/// Dispatches EPT invalidation and retains the frames until every CPU has flushed.
///
/// The current CPU flushes before returning; remote CPUs flush asynchronously.
/// Each CPU owns a reference to every frame until its INVEPT completes.
/// The returned handle can be used to wait for remote invalidations to complete.
///
/// # Panics
///
/// Panics if EPT is unsupported or INVEPT fails on the current CPU.
pub(crate) fn invalidate(
    _vmx_guard: &VmxGuard,
    frames: &[RcuDrop<Frame<dyn AnyFrameMeta>>],
) -> PendingIpis {
    assert!(is_ept_supported());
    let targets = CpuSet::new_full();

    for cpu in targets.iter() {
        let mut queue = PENDING_INVALIDATION.get_on_cpu(cpu).lock();
        queue
            .frames
            .extend(frames.iter().map(|frame| (**frame).clone()));
        queue.pending = true;
    }

    smp::inter_processor_call(&targets, flush_pending)
}

pub(super) fn flush_pending() {
    let irq_guard = irq::disable_local();
    let frames = {
        let local_queue = PENDING_INVALIDATION.get_with(&irq_guard);
        let mut queue = local_queue.lock();
        if !queue.pending {
            return;
        }

        // SAFETY:
        // Requests are queued while
        // 1. a `VmxGuard` is alive,
        // 2. after checking INVEPT support.
        // VMX teardown calls this function before VMXOFF, so any later queued
        // callbacks find no pending work and do not execute INVEPT.
        unsafe { instructions::invept_all_contexts(&irq_guard) }.unwrap();
        queue.pending = false;
        core::mem::take(&mut queue.frames)
    };
    drop(frames);
}
