// SPDX-License-Identifier: MPL-2.0

use x86::{
    apic::xapic,
    msr::{rdmsr, wrmsr, IA32_APIC_BASE},
};

use super::ApicTimer;
use crate::{
    io::{IoMem, Sensitive},
    mm::{HasPaddr, HasSize},
};

#[derive(Debug)]
pub(super) struct XApic {
    io_mem: IoMem<Sensitive>,
}

// The APIC instance can be shared among threads running on the same CPU, but not among those
// running on different CPUs. Therefore, it is not `Send`/`Sync`.
impl !Send for XApic {}
impl !Sync for XApic {}

impl XApic {
    pub(super) fn new(io_mem: &IoMem<Sensitive>) -> Option<Self> {
        if !Self::has_xapic() {
            return None;
        }

        // SAFETY: xAPIC is present.
        assert_eq!(io_mem.paddr(), unsafe { read_xapic_base_address() });
        assert_eq!(io_mem.size(), XAPIC_MMIO_SIZE);
        Some(Self {
            io_mem: io_mem.clone(),
        })
    }

    pub(super) fn has_xapic() -> bool {
        use crate::arch::cpu::extension::{has_extensions, IsaExtensions};

        has_extensions(IsaExtensions::XAPIC)
    }

    pub(super) fn enable(&mut self) {
        const XAPIC_ENABLE_BITS: u64 = {
            // IA32_APIC_BASE MSR's EN bit: xAPIC global enable/disable
            const EN_BIT_IDX: u8 = 11;

            1 << EN_BIT_IDX
        };

        // SAFETY: These operations enable xAPIC, which is safe because `XApic` will only be
        // constructed if xAPIC is known to be present.
        unsafe {
            // Enable xAPIC if it is not enabled by default.
            wrmsr(
                IA32_APIC_BASE,
                self.io_mem.paddr() as u64 | XAPIC_ENABLE_BITS,
            );

            // Set SVR. Enable APIC and set Spurious Vector to 15 (reserved IRQ number).
            let svr: u32 = (1 << 8) | 15;
            self.io_mem.write_once(xapic::XAPIC_SVR as usize, &svr);
        }
    }
}

impl super::Apic for XApic {
    fn id(&self) -> u32 {
        unsafe { self.io_mem.read_once(xapic::XAPIC_ID as usize) }
    }

    fn version(&self) -> u32 {
        unsafe { self.io_mem.read_once(xapic::XAPIC_VERSION as usize) }
    }

    fn eoi(&self) {
        unsafe { self.io_mem.write_once(xapic::XAPIC_EOI as usize, &0u32) };
    }

    unsafe fn send_ipi(&self, icr: super::Icr) {
        let _guard = crate::irq::disable_local();

        // SAFETY: These operations write the interrupt command to APIC and wait for results. The
        // caller guarantees it's safe to execute this interrupt command.
        unsafe {
            self.io_mem.write_once(xapic::XAPIC_ESR as usize, &0u32);
            // The upper 32 bits of ICR must be written into XAPIC_ICR1 first,
            // because writing into XAPIC_ICR0 will trigger the action of
            // interrupt sending.
            self.io_mem
                .write_once(xapic::XAPIC_ICR1 as usize, &icr.upper());
            self.io_mem
                .write_once(xapic::XAPIC_ICR0 as usize, &icr.lower());
            loop {
                let icr = self.io_mem.read_once::<u32>(xapic::XAPIC_ICR0 as usize);
                if ((icr >> 12) & 0x1) == 0 {
                    break;
                }
                if self.io_mem.read_once::<u32>(xapic::XAPIC_ESR as usize) > 0 {
                    break;
                }
            }
        }
    }
}

impl ApicTimer for XApic {
    fn set_timer_init_count(&self, value: u64) {
        unsafe {
            self.io_mem
                .write_once(xapic::XAPIC_TIMER_INIT_COUNT as usize, &(value as u32))
        };
    }

    fn timer_current_count(&self) -> u64 {
        unsafe {
            self.io_mem
                .read_once::<u32>(xapic::XAPIC_TIMER_CURRENT_COUNT as usize) as u64
        }
    }

    fn set_lvt_timer(&self, value: u64) {
        unsafe {
            self.io_mem
                .write_once(xapic::XAPIC_LVT_TIMER as usize, &(value as u32))
        };
    }

    fn set_timer_div_config(&self, div_config: super::DivideConfig) {
        unsafe {
            self.io_mem
                .write_once(xapic::XAPIC_TIMER_DIV_CONF as usize, &(div_config as u32))
        };
    }
}

/// Reads xAPIC base address from the MSR.
///
/// # Safety
///
/// The caller must ensure that xAPIC is present.
pub(super) unsafe fn read_xapic_base_address() -> usize {
    (unsafe { rdmsr(IA32_APIC_BASE) & 0xffff_f000 }) as usize
}

/// The size of the xAPIC MMIO region.
pub(super) const XAPIC_MMIO_SIZE: usize = size_of::<[u32; 256]>();
