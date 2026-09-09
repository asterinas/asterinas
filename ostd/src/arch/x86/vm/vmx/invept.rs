// SPDX-License-Identifier: MPL-2.0

use x86::{
    msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr},
    vmx::vmcs::control::{PrimaryControls, SecondaryControls},
};

use super::{VmxGuard, instructions};
use crate::{
    Error,
    cpu::CpuSet,
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, frame::meta::AnyFrameMeta},
    prelude::*,
    smp::{self, PendingIpis},
    sync::{LocalIrqDisabled, RcuDrop, SpinLock},
};

struct PendingInvalidation {
    /// Whether invalidation is pending.
    ///
    /// Note that permission changes can set this without frames to be released.
    is_pending: bool,
    /// Frames that will only be released after INVEPT.
    frames: Vec<Frame<dyn AnyFrameMeta>>,
}

cpu_local! {
    static PENDING_INVALIDATION: SpinLock<PendingInvalidation, LocalIrqDisabled> =
        SpinLock::new(PendingInvalidation {
            is_pending: false,
            frames: Vec::new(),
        });
}

/// Checks the EPT and invalidation capabilities used by guest memory.
///
/// # Safety
///
/// The current CPU must support VMX.
pub(super) unsafe fn check_ept_support(_irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    // SAFETY: The caller guarantees VMX support.
    let primary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS) } >> 32) as u32;
    if primary_allowed_1 & PrimaryControls::SECONDARY_CONTROLS.bits() == 0 {
        return Err(Error::NotEnoughResources);
    }
    // SAFETY: The primary-control check established secondary-control support.
    let secondary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS2) } >> 32) as u32;
    if secondary_allowed_1 & SecondaryControls::ENABLE_EPT.bits() == 0 {
        return Err(Error::NotEnoughResources);
    }

    // SAFETY: The secondary-control check established EPT support.
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
        return Err(Error::NotEnoughResources);
    }
    Ok(())
}

/// Dispatches EPT invalidation and retains the frames until every CPU has flushed.
///
/// The current CPU flushes before returning; remote CPUs flush asynchronously.
/// Each CPU owns a reference to every frame until its INVEPT completes.
/// The returned handle can be used to wait for remote invalidations to complete.
pub(crate) fn invalidate(
    _vmx_guard: &VmxGuard,
    frames: &[RcuDrop<Frame<dyn AnyFrameMeta>>],
) -> PendingIpis {
    let targets = CpuSet::new_full();

    for cpu in targets.iter() {
        let mut queue = PENDING_INVALIDATION.get_on_cpu(cpu).lock();
        queue
            .frames
            .extend(frames.iter().map(|frame| (**frame).clone()));
        queue.is_pending = true;
    }

    smp::inter_processor_call(&targets, flush_pending)
}

pub(super) fn flush_pending() {
    let irq_guard = irq::disable_local();
    let frames = {
        let local_queue = PENDING_INVALIDATION.get_with(&irq_guard);
        let mut queue = local_queue.lock();
        if !queue.is_pending {
            return;
        }

        // SAFETY:
        // 1. Requests are queued under a `VmxGuard`. VMX teardown drains the
        //    queue before `VMXOFF`, so later callbacks find no pending work.
        // 2. The guard guarantees all-context INVEPT support on every CPU.
        unsafe { instructions::invept_all_contexts(&irq_guard) }.unwrap();
        queue.is_pending = false;
        core::mem::take(&mut queue.frames)
    };
    drop(frames);
}
