// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupts.
//!
//! IPIs are delivered through GIC software-generated interrupt (SGI) 0.

use crate::{arch::irq::IRQ_CHIP, cpu::PinCurrentCpu};

/// The SGI interrupt ID used for general IPIs.
pub(in crate::arch) const IPI_SGI: u8 = 0;

/// Hardware-specific, architecture-dependent CPU ID.
///
/// On AArch64 this is the GIC CPU-interface number, which QEMU `virt` derives
/// from `MPIDR_EL1.Aff0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        Self(crate::arch::boot::smp::get_current_hart_id())
    }
}

/// Initializes the IPI state on the BSP.
///
/// # Safety
///
/// This function can only be called on the BSP and before any other IPI-related
/// function is called.
pub(in crate::arch) unsafe fn init_on_bsp() {
    if let Some(chip) = IRQ_CHIP.get() {
        chip.enable(IPI_SGI);
    }
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) {
    if let Some(chip) = IRQ_CHIP.get() {
        chip.send_sgi(IPI_SGI, hw_cpu_id.0 as u8);
    }
}
