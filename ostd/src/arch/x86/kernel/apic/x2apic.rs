// SPDX-License-Identifier: MPL-2.0

use x86::msr::{
    IA32_APIC_BASE, IA32_X2APIC_APICID, IA32_X2APIC_CUR_COUNT, IA32_X2APIC_DIV_CONF,
    IA32_X2APIC_EOI, IA32_X2APIC_ESR, IA32_X2APIC_ICR, IA32_X2APIC_INIT_COUNT,
    IA32_X2APIC_LVT_TIMER, IA32_X2APIC_SIVR, IA32_X2APIC_VERSION, rdmsr, wrmsr,
};

use super::ApicTimer;

#[derive(Debug)]
pub(super) struct X2Apic {
    _private: (),
}

// The APIC instance can be shared among threads running on the same CPU, but not among those
// running on different CPUs. Therefore, it is not `Send`/`Sync`.
impl !Send for X2Apic {}
impl !Sync for X2Apic {}

impl X2Apic {
    pub(super) fn new() -> Option<Self> {
        if !Self::has_x2apic() {
            return None;
        }

        Some(Self { _private: () })
    }

    pub(super) fn has_x2apic() -> bool {
        use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

        has_extensions(IsaExtensions::X2APIC)
    }

    pub(super) fn enable(&mut self) {
        const X2APIC_ENABLE_BITS: u64 = {
            // IA32_APIC_BASE MSR's EN bit: xAPIC global enable/disable
            const EN_BIT_IDX: u8 = 11;
            // IA32_APIC_BASE MSR's EXTD bit: Enable x2APIC mode
            const EXTD_BIT_IDX: u8 = 10;

            (1 << EN_BIT_IDX) | (1 << EXTD_BIT_IDX)
        };

        // SAFETY: These operations enable x2APIC, which is safe because `X2Apic` will only be
        // constructed if x2APIC is known to be present.
        unsafe {
            // Enable x2APIC and xAPIC if they are not enabled by default.
            let mut base = rdmsr(IA32_APIC_BASE);
            if base & X2APIC_ENABLE_BITS != X2APIC_ENABLE_BITS {
                base |= X2APIC_ENABLE_BITS;
                wrmsr(IA32_APIC_BASE, base);
            }

            // Set SVR. Enable APIC and set Spurious Vector to 15 (reserved IRQ number).
            let svr: u64 = (1 << 8) | 15;
            wrmsr(IA32_X2APIC_SIVR, svr);
        }
    }
}

impl super::Apic for X2Apic {
    fn id(&self) -> u32 {
        unsafe { rdmsr(IA32_X2APIC_APICID) as u32 }
    }

    fn version(&self) -> u32 {
        unsafe { rdmsr(IA32_X2APIC_VERSION) as u32 }
    }

    fn eoi(&self) {
        unsafe { wrmsr(IA32_X2APIC_EOI, 0) };
    }

    unsafe fn send_ipi(&self, icr: super::Icr) {
        let _guard = crate::irq::disable_local();

        // SAFETY: These operations write the interrupt command to APIC and wait for results. The
        // caller guarantees it's safe to execute this interrupt command.
        unsafe {
            wrmsr(IA32_X2APIC_ESR, 0);
            wrmsr(IA32_X2APIC_ICR, icr.0);
            loop {
                let icr = rdmsr(IA32_X2APIC_ICR);
                if ((icr >> 12) & 0x1) == 0 {
                    break;
                }
                if rdmsr(IA32_X2APIC_ESR) > 0 {
                    break;
                }
            }
        }
    }
}

impl ApicTimer for X2Apic {
    fn set_timer_init_count(&self, value: u64) {
        unsafe { wrmsr(IA32_X2APIC_INIT_COUNT, value) };
    }

    fn timer_current_count(&self) -> u64 {
        unsafe { rdmsr(IA32_X2APIC_CUR_COUNT) }
    }

    fn set_lvt_timer(&self, value: u64) {
        unsafe { wrmsr(IA32_X2APIC_LVT_TIMER, value) };
    }

    fn set_timer_div_config(&self, div_config: super::DivideConfig) {
        unsafe { wrmsr(IA32_X2APIC_DIV_CONF, div_config as u64) };
    }
}
