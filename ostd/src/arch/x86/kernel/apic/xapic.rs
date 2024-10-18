// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use x86::apic::xapic;

use super::ApicTimer;
use crate::mm;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const IA32_APIC_BASE_MSR_BSP: u32 = 0x100; // Processor is a BSP
const IA32_APIC_BASE_MSR_ENABLE: u64 = 0x800;

const APIC_LVT_MASK_BITS: u32 = 1 << 16;

#[derive(Debug)]
pub struct XApic {
    mmio_start: *mut u32,
}

impl XApic {
    pub fn new() -> Option<Self> {
        if !Self::has_xapic() {
            return None;
        }
        let address = mm::paddr_to_vaddr(get_apic_base_address());
        Some(Self {
            mmio_start: address as *mut u32,
        })
    }

    /// Reads a register from the MMIO region.
    fn read(&self, offset: u32) -> u32 {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        debug_assert!(index < 256);
        unsafe { core::ptr::read_volatile(self.mmio_start.add(index)) }
    }

    /// Writes a register in the MMIO region.
    fn write(&self, offset: u32, val: u32) {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        debug_assert!(index < 256);
        unsafe { core::ptr::write_volatile(self.mmio_start.add(index), val) }
    }

    pub fn enable(&mut self) {
        // Enable xAPIC
        set_apic_base_address(get_apic_base_address());

        // Set SVR, Enable APIC and set Spurious Vector to 15 (Reserved irq number)
        let svr: u32 = 1 << 8 | 15;
        self.write(xapic::XAPIC_SVR, svr);
    }

    pub(super) fn has_xapic() -> bool {
        let value = unsafe { core::arch::x86_64::__cpuid(1) };
        value.edx & 0x100 != 0
    }
}

impl super::Apic for XApic {
    fn id(&self) -> u32 {
        self.read(xapic::XAPIC_ID)
    }

    fn version(&self) -> u32 {
        self.read(xapic::XAPIC_VERSION)
    }

    fn eoi(&self) {
        self.write(xapic::XAPIC_EOI, 0);
    }

    unsafe fn send_ipi(&self, icr: super::Icr) {
        let _guard = crate::trap::disable_local();
        self.write(xapic::XAPIC_ESR, 0);
        // The upper 32 bits of ICR must be written into XAPIC_ICR1 first,
        // because writing into XAPIC_ICR0 will trigger the action of
        // interrupt sending.
        self.write(xapic::XAPIC_ICR1, icr.upper());
        self.write(xapic::XAPIC_ICR0, icr.lower());
        loop {
            let icr = self.read(xapic::XAPIC_ICR0);
            if (icr >> 12 & 0x1) == 0 {
                break;
            }
            if self.read(xapic::XAPIC_ESR) > 0 {
                break;
            }
        }
    }
}

impl ApicTimer for XApic {
    fn set_timer_init_count(&self, value: u64) {
        self.write(xapic::XAPIC_TIMER_INIT_COUNT, value as u32);
    }

    fn timer_current_count(&self) -> u64 {
        self.read(xapic::XAPIC_TIMER_CURRENT_COUNT) as u64
    }

    fn set_lvt_timer(&self, value: u64) {
        self.write(xapic::XAPIC_LVT_TIMER, value as u32);
    }

    fn set_timer_div_config(&self, div_config: super::DivideConfig) {
        self.write(xapic::XAPIC_TIMER_DIV_CONF, div_config as u32);
    }
}

/// Sets APIC base address and enables it
fn set_apic_base_address(address: usize) {
    unsafe {
        x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR)
            .write(address as u64 | IA32_APIC_BASE_MSR_ENABLE);
    }
}

/// Gets APIC base address
fn get_apic_base_address() -> usize {
    unsafe {
        (x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR).read() & 0xf_ffff_f000)
            as usize
    }
}
