// SPDX-License-Identifier: MPL-2.0

use x86::msr::{
    rdmsr, wrmsr, IA32_APIC_BASE, IA32_X2APIC_APICID, IA32_X2APIC_CUR_COUNT, IA32_X2APIC_DIV_CONF,
    IA32_X2APIC_EOI, IA32_X2APIC_ESR, IA32_X2APIC_ICR, IA32_X2APIC_INIT_COUNT,
    IA32_X2APIC_LVT_TIMER, IA32_X2APIC_SIVR, IA32_X2APIC_VERSION,
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
        use crate::arch::cpu::extension::{has_extensions, IsaExtensions};

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
            // TODO: Prevent Iago attack: Avoid accessing this register in Intel TDX environment.
            // This RDMSR triggers #VE exception, and in Intel TDX, x2APIC is enabled by default.
            // Malicious VMM/Host could exploit this access to:
            // - Invalid APIC base address pointing to critical memory regions or device MMIO
            // - Corrupted enable bits that could cause undefined processor behavior
            // Consider implementing: In Intel TDX environments, X2APIC is enabled by default,
            // so avoid accessing this register.
            let mut base = rdmsr(IA32_APIC_BASE);
            if base & X2APIC_ENABLE_BITS != X2APIC_ENABLE_BITS {
                base |= X2APIC_ENABLE_BITS;
                wrmsr(IA32_APIC_BASE, base);
            }

            // Set SVR. Enable APIC and set Spurious Vector to 15 (reserved IRQ number).
            // TODO: Prevent Iago attack: Verify SVR write operation succeeded in Intel TDX environment.
            // This WRMSR triggers #VE exception, delegating SVR configuration to untrusted VMM.
            // A malicious VMM/Host could exploit this to:
            // - Silently ignore or modify the written value
            // - Return false success while blocking the actual configuration
            // - Partially tamper with written values (e.g., disable APIC while keeping vector number)
            // Consider implementing: immediate readback verification and functional verification
            // to ensure APIC is actually enabled.
            let svr: u64 = (1 << 8) | 15;
            wrmsr(IA32_X2APIC_SIVR, svr);
        }
    }
}

impl super::Apic for X2Apic {
    fn id(&self) -> u32 {
        // TODO: Prevent Iago attack: Validate APIC ID value and detect VMM tampering in Intel TDX environment.
        // This RDMSR triggers #VE exception, delegating APIC ID retrieval to untrusted VMM.
        // A malicious VMM/Host could exploit this to:
        // - VMM/Host interference causing inconsistent ID reads across multiple calls
        // - Malicious ID values designed to disrupt CPU mapping or IPI routing
        // This ID is used for critical operations: CPU identification, IPI targeting, and
        // hardware CPU mapping. Consider implementing: range validation against known CPU
        // topology, consistency checks across multiple reads, cross-validation with CPUID
        // and other CPU identification methods, and detection of VMM interference patterns.
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
            // TODO: Prevent Iago attack: Verify IPI delivery and detect VMM/Host interference in Intel TDX environment.
            // These two WRMSRs trigger #VE exceptions, delegating IPI delivery to untrusted VMM.
            // Malicious VMM/Host can interfere with IPI delivery:
            // - IPIs may be silently dropped or delayed by malicious hypervisor control
            // - ESR values could be manipulated to hide delivery failures
            // - ICR delivery status bit may be controlled to fake successful delivery
            // - Infinite loops possible if VMM prevents delivery status from clearing
            // Consider implementing: timeout-based delivery verification, ESR validation against
            // known error patterns, delivery status cross-validation, and fail-fast mechanisms
            // when IPI delivery is compromised to prevent system hangs or security violations.
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
        // TODO: Prevent Iago attack: Validate LVT Timer configuration in TDX environment.
        // This WRMSR triggers #VE exception, delegating timer configuration to untrusted VMM.
        // It can be exploited by malicious VMM/Host to tamper with timer configuration:
        // - Silently block or modify timer mode bits (periodic/deadline)
        // - Tamper with interrupt vector numbers causing timer interrupts to be misrouted
        // - Manipulate mask bit to disable timer interrupts leading to system hangs
        // - Modify delivery mode causing timer to use wrong interrupt delivery mechanism
        // - Set reserved bits violating APIC specification and causing undefined behavior
        // Timer compromise can result in: scheduling failures, system hangs, security bypass
        // through timing attacks, or complete system unresponsiveness.
        // Consider implementing: readback verification to detect write tampering, timer
        // functionality testing, and fail-fast mechanisms when timer integrity is compromised.
        unsafe { wrmsr(IA32_X2APIC_LVT_TIMER, value) };
    }

    fn set_timer_div_config(&self, div_config: super::DivideConfig) {
        unsafe { wrmsr(IA32_X2APIC_DIV_CONF, div_config as u64) };
    }
}
