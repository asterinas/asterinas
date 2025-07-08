// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

use crate::cpu::PinCurrentCpu;

pub(crate) const IRQ_NUM_MIN: u8 = 32;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

pub(crate) struct IrqRemapping {
    _private: (),
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    /// Initializes the remapping entry for the specific IRQ number.
    ///
    /// This will do nothing if the entry is already initialized or interrupt
    /// remapping is disabled or not supported by the architecture.
    pub(crate) fn init(&self, irq_num: u8) {}

    /// Gets the remapping index of the IRQ line.
    ///
    /// This method will return `None` if interrupt remapping is disabled or
    /// not supported by the architecture.
    pub(crate) fn remapping_index(&self) -> Option<u16> {
        None
    }
}

pub(crate) fn enable_local() {
    loongArch64::register::crmd::set_ie(true);
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
//
// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local_and_halt() {
    loongArch64::register::crmd::set_ie(true);
    // TODO: We should put the CPU into the idle state. However, doing so
    // without creating race conditions (see the doc comments above) in
    // LoongArch is challenging. Therefore, we now simply return here, as
    // spurious wakeups are acceptable for this method.
}

pub(crate) fn disable_local() {
    loongArch64::register::crmd::set_ie(false);
}

pub(crate) fn is_local_enabled() -> bool {
    loongArch64::register::crmd::read().ie()
}

// ####### Inter-Processor Interrupts (IPIs) #######

/// Hardware-specific, architecture-dependent CPU ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(guard: &dyn PinCurrentCpu) -> Self {
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
pub(crate) unsafe fn send_ipi(hw_cpu_id: HwCpuId, irq_num: u8, guard: &dyn PinCurrentCpu) {
    unimplemented!()
}
