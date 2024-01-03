// SPDX-License-Identifier: MPL-2.0

use x86::msr::{
    rdmsr, wrmsr, IA32_APIC_BASE, IA32_X2APIC_APICID, IA32_X2APIC_CUR_COUNT, IA32_X2APIC_DIV_CONF,
    IA32_X2APIC_EOI, IA32_X2APIC_INIT_COUNT, IA32_X2APIC_LVT_TIMER, IA32_X2APIC_SIVR,
    IA32_X2APIC_VERSION,
};

use super::ApicTimer;

pub struct X2Apic {}

impl X2Apic {
    pub(crate) fn new() -> Option<Self> {
        if !Self::has_x2apic() {
            return None;
        }
        Some(Self {})
    }

    fn has_x2apic() -> bool {
        // x2apic::X2APIC::new()
        let value = unsafe { core::arch::x86_64::__cpuid(1) };
        value.ecx & 0x20_0000 != 0
    }

    pub fn enable(&mut self) {
        // Enable
        unsafe {
            // Enable x2APIC mode globally
            let mut base = rdmsr(IA32_APIC_BASE);
            base |= 0b1100_0000_0000; // Enable x2APIC and xAPIC
            wrmsr(IA32_APIC_BASE, base);

            // Set SVR, Enable APIC and set Spurious Vector to 15 (Reserved irq number)
            let svr: u64 = 1 << 8 | 15;
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

    fn eoi(&mut self) {
        unsafe {
            wrmsr(IA32_X2APIC_EOI, 0);
        }
    }
}

impl ApicTimer for X2Apic {
    fn set_timer_init_count(&mut self, value: u64) {
        unsafe {
            wrmsr(IA32_X2APIC_INIT_COUNT, value);
        }
    }

    fn timer_current_count(&self) -> u64 {
        unsafe { rdmsr(IA32_X2APIC_CUR_COUNT) }
    }

    fn set_lvt_timer(&mut self, value: u64) {
        unsafe {
            wrmsr(IA32_X2APIC_LVT_TIMER, value);
        }
    }

    fn set_timer_div_config(&mut self, div_config: super::DivideConfig) {
        unsafe {
            wrmsr(IA32_X2APIC_DIV_CONF, div_config as u64);
        }
    }
}
