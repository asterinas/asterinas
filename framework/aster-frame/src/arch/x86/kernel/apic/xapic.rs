use crate::sync::Mutex;
use crate::vm;
use spin::Once;
use x86::apic::xapic;

use super::ApicTimer;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const IA32_APIC_BASE_MSR_BSP: u32 = 0x100; // Processor is a BSP
const IA32_APIC_BASE_MSR_ENABLE: u64 = 0x800;

const APIC_LVT_MASK_BITS: u32 = 1 << 16;

pub static XAPIC_INSTANCE: Once<Mutex<XApic>> = Once::new();

#[derive(Debug)]
pub struct XApic {
    mmio_region: &'static mut [u32],
}

impl XApic {
    pub fn new() -> Option<Self> {
        if !Self::has_xapic() {
            return None;
        }
        let address = vm::paddr_to_vaddr(get_apic_base_address());
        let region: &'static mut [u32] = unsafe { &mut *(address as *mut [u32; 256]) };
        Some(Self {
            mmio_region: region,
        })
    }

    /// Read a register from the MMIO region.
    fn read(&self, offset: u32) -> u32 {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        unsafe { core::ptr::read_volatile(&self.mmio_region[index]) }
    }

    /// write a register in the MMIO region.
    fn write(&mut self, offset: u32, val: u32) {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        unsafe { core::ptr::write_volatile(&mut self.mmio_region[index], val) }
    }

    pub fn enable(&mut self) {
        // Enable xAPIC
        set_apic_base_address(get_apic_base_address());

        // Set SVR, Enable APIC and set Spurious Vector to 15 (Reserved irq number)
        let svr: u32 = 1 << 8 | 15;
        self.write(xapic::XAPIC_SVR, svr);
    }

    pub fn has_xapic() -> bool {
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

    fn eoi(&mut self) {
        self.write(xapic::XAPIC_EOI, 0);
    }
}

impl ApicTimer for XApic {
    fn set_timer_init_count(&mut self, value: u64) {
        self.write(xapic::XAPIC_TIMER_INIT_COUNT, value as u32);
    }

    fn timer_current_count(&self) -> u64 {
        self.read(xapic::XAPIC_TIMER_CURRENT_COUNT) as u64
    }

    fn set_lvt_timer(&mut self, value: u64) {
        self.write(xapic::XAPIC_LVT_TIMER, value as u32);
    }

    fn set_timer_div_config(&mut self, div_config: super::DivideConfig) {
        self.write(xapic::XAPIC_TIMER_DIV_CONF, div_config as u32);
    }
}

/// set APIC base address and enable it
fn set_apic_base_address(address: usize) {
    unsafe {
        x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR)
            .write(address as u64 | IA32_APIC_BASE_MSR_ENABLE);
    }
}

/// get APIC base address
fn get_apic_base_address() -> usize {
    unsafe {
        (x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR).read() & 0xf_ffff_f000)
            as usize
    }
}
