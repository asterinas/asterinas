// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupt support.

use spin::Once;

use crate::{cpu::PinCurrentCpu, irq::IrqLine};

/// Hardware CPU identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct HwCpuId(u64);

impl HwCpuId {
    /// Reads the current hardware CPU ID from MPIDR_EL1.
    ///
    /// MPIDR_EL1 encodes the CPU affinity:
    /// - Aff0 (bits [7:0]):   core ID within a cluster
    /// - Aff1 (bits [15:8]):  cluster ID
    /// - Aff2 (bits [23:16]): cluster ID (multi-level)
    /// - Aff3 (bits [31:24]): affinity level 3
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        let mpidr: u64;
        // SAFETY: Reading MPIDR_EL1 is always safe.
        unsafe { core::arch::asm!("mrs {0}, mpidr_el1", out(reg) mpidr) };
        HwCpuId(mpidr & 0xff_ffff) // Use Aff0-Aff2 (24 bits)
    }
}

/// The IPI IRQ line.
pub(crate) static IPI_IRQ: Once<IrqLine> = Once::new();

/// Sends an IPI to the given hardware CPU via GICv3 SGI.
///
/// Writes ICC_SGI1R_EL1 to generate a Software Generated Interrupt (SGI)
/// targeting the specified CPU.
pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) {
    // ICC_SGI1R_EL1: Interrupt Controller Software Generated Interrupt Group 1 Register
    // - TargetList (bits [23:16]): affinity 0 target list (1 bit per core)
    // - aff1 (bits [39:32]): affinity level 1
    // - aff2 (bits [55:48]): affinity level 2
    // - aff3 (bits [63:56]): affinity level 3
    // - INTID (bits [27:24]): interrupt ID (0-15 for SGIs)
    // - IRM (bit 40): interrupt routing mode (0 = use TargetList)
    let mpidr = hw_cpu_id.0;
    let aff0 = mpidr & 0xff;
    let aff1 = (mpidr >> 8) & 0xff;
    let aff2 = (mpidr >> 16) & 0xff;
    let aff3 = (mpidr >> 32) & 0xff;

    // TargetList: set the bit corresponding to the target core
    let target_list: u64 = 1 << aff0;

    // SGI INTID 0 is used for IPIs
    let intid: u64 = 0;

    let sgi1r = (aff3 << 48) | (aff2 << 32) | (aff1 << 16) | (target_list) | (intid << 24);

    // SAFETY: Writing ICC_SGI1R_EL1 from EL1 is safe.
    unsafe {
        core::arch::asm!("msr icc_sgi1r_el1, {0}", in(reg) sgi1r);
    }
}

/// Initializes the IPI module on the BSP.
///
/// # Safety
///
/// Must be called only once on BSP before any IPI is sent.
pub(crate) unsafe fn init_on_bsp() {
    let mut irq = IrqLine::alloc().unwrap();
    // SAFETY: This will be called upon an inter-processor interrupt.
    irq.on_active(|f| unsafe { crate::smp::do_inter_processor_call(f) });
    IPI_IRQ.call_once(|| irq);
}

/// Initializes the IPI module on an AP.
///
/// # Safety
///
/// Must be called only once on each AP.
pub(crate) unsafe fn init_on_ap() {
    // SGIs are always enabled and don't need per-CPU initialization.
    // The AP's GIC CPU interface is already configured during AP boot.
}
