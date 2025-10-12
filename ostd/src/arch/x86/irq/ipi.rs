// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupts.

use crate::cpu::PinCurrentCpu;

/// Hardware-specific, architecture-dependent CPU ID.
///
/// This is the Local APIC ID in the x86_64 architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(guard: &dyn PinCurrentCpu) -> Self {
        use crate::arch::kernel::apic;

        let apic = apic::get_or_init(guard);
        Self(apic.id())
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
    use crate::arch::kernel::apic::{self, Icr};

    let icr = Icr::new(
        apic::ApicId::from(hw_cpu_id.0),
        apic::DestinationShorthand::NoShorthand,
        apic::TriggerMode::Edge,
        apic::Level::Assert,
        apic::DeliveryStatus::Idle,
        apic::DestinationMode::Physical,
        apic::DeliveryMode::Fixed,
        irq_num,
    );

    let apic = apic::get_or_init(guard);
    // SAFETY: The ICR is valid to generate the request IPI. Generating the request IPI is safe
    // as guaranteed by the caller.
    unsafe { apic.send_ipi(icr) };
}
