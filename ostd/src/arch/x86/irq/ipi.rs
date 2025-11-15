// SPDX-License-Identifier: MPL-2.0

//! Inter-processor interrupts.

use spin::Once;

use crate::{cpu::PinCurrentCpu, irq::IrqLine, smp::do_inter_processor_call};

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

static IPI_IRQ: Once<IrqLine> = Once::new();

/// Initializes global IPI state.
pub(in crate::arch) fn init() {
    let mut irq = IrqLine::alloc().unwrap();
    // SAFETY: This will be called upon an inter-processor interrupt.
    irq.on_active(|f| unsafe { do_inter_processor_call(f) });
    IPI_IRQ.call_once(|| irq);
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, guard: &dyn PinCurrentCpu) {
    use crate::arch::kernel::apic::{self, Icr};

    let irq_num = IPI_IRQ.get().unwrap().num();

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
    // SAFETY: The ICR is valid to generate the request IPI. Generating the
    // request IPI is safe.
    unsafe { apic.send_ipi(icr) };
}
