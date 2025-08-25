// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupts.

use crate::cpu::PinCurrentCpu;

/// Hardware-specific, architecture-dependent CPU ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        // TODO: Support SMP in LoongArch.
        Self(0)
    }
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
///
/// # Safety
///
/// The caller must ensure that the interrupt number is valid and that
/// the corresponding handler is configured correctly on the remote CPU.
/// Furthermore, invoking the interrupt handler must also be safe.
pub(crate) unsafe fn send_ipi(_hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) {
    // To suppress unused function lint errors. We should be using it here.
    let _ = crate::smp::do_inter_processor_call;
    unimplemented!()
}
