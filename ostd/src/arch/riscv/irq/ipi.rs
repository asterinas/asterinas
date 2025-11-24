// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupts.

use spin::Once;

use crate::{cpu::PinCurrentCpu, irq::IrqLine};

/// Hardware-specific, architecture-dependent CPU ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        // No races because of `_guard`.
        Self(crate::arch::boot::smp::get_current_hart_id())
    }
}

pub(in crate::arch) static IPI_IRQ: Once<IrqLine> = Once::new();

/// Initializes the global IPI-related state and local state on the BSP.
///
/// # Safety
///
/// This function can only be called on the BSP and before any other
/// IPI-related function is called.
pub(in crate::arch) unsafe fn init_on_bsp() {
    let mut irq = IrqLine::alloc().unwrap();
    // SAFETY: This will be called upon an inter-processor interrupt.
    irq.on_active(|f| unsafe { crate::smp::do_inter_processor_call(f) });
    IPI_IRQ.call_once(|| irq);

    // SAFETY: Enabling the software interrupts is safe here because this
    // function cannot be called when others can perform IPI-related
    // operations. And it has no side-effects.
    unsafe { riscv::register::sie::set_ssoft() };
}

/// Initializes the IPI-related state on this AP.
///
/// # Safety
///
/// This function can only be called before any other harts can send IPIs to
/// this application hart.
pub(in crate::arch) unsafe fn init_on_ap() {
    // SAFETY: Enabling the software interrupts is safe here due to the same
    // reasons mentioned in `init`.
    unsafe { riscv::register::sie::set_ssoft() };
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) {
    const XLEN: usize = usize::BITS as usize;
    const XLEN_MASK: usize = XLEN - 1;

    let hart_id = hw_cpu_id.0 as usize;
    let hart_mask_base = hart_id & !XLEN_MASK;
    let hart_mask = 1 << (hart_id & XLEN_MASK);

    let ret = sbi_rt::send_ipi(sbi_rt::HartMask::from_mask_base(hart_mask, hart_mask_base));

    if ret.error == 0 {
        log::debug!("Successfully sent IPI to hart {}", hw_cpu_id.0);
    } else {
        log::error!(
            "Failed to send IPI to hart {}: error code {}",
            hw_cpu_id.0,
            ret.error
        );
    }
}
