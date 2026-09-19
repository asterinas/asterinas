// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use x86::msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr};

use super::{VmxGuard, instructions};
use crate::{
    cpu::CpuSet,
    cpu_local, irq,
    mm::{Frame, frame::meta::AnyFrameMeta},
    prelude::*,
    smp,
    sync::{LocalIrqDisabled, RcuDrop, SpinLock},
};

pub(crate) static SUPPORT_EPT: AtomicBool = AtomicBool::new(false);

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

/// Dispatches EPT invalidation and retains the frames until every CPU has flushed.
///
/// The current CPU flushes before returning; remote CPUs flush asynchronously.
/// Each CPU owns a reference to every frame until its INVEPT completes, and the
/// references are then dropped after an RCU grace period.
///
/// # Panics
///
/// Panics if `SUPPORT_EPT` is false or INVEPT fails on the current CPU.
pub(crate) fn invalidate(_vmx_guard: &VmxGuard, frames: &[RcuDrop<Frame<dyn AnyFrameMeta>>]) {
    assert!(SUPPORT_EPT.load(Ordering::Relaxed));
    let _irq_guard = irq::disable_local();
    let targets = CpuSet::new_full();

    for cpu in targets.iter() {
        let mut queue = PENDING_INVALIDATION.get_on_cpu(cpu).lock();
        queue
            .frames
            .extend(frames.iter().map(|frame| (**frame).clone()));
        queue.pending = true;
    }

    let _ = smp::inter_processor_call(&targets, flush_pending);
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
        // 1. a VmxGuard is alive,
        // 2. after checking INVEPT support.
        // VMX teardown calls this function before VMXOFF, so any later queued
        // callbacks find no pending work and do not execute INVEPT.
        unsafe { instructions::invept_all_contexts(&irq_guard) }.unwrap();
        queue.pending = false;
        core::mem::take(&mut queue.frames)
    };
    // Release references after both INVEPT and RCU.
    drop(RcuDrop::new(frames));
}
