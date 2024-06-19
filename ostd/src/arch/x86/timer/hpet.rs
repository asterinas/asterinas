// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::vec::Vec;

use acpi::{AcpiError, HpetInfo};
use spin::Once;
use volatile::{
    access::{ReadOnly, ReadWrite},
    Volatile,
};

use crate::{
    arch::x86::kernel::{acpi::ACPI_TABLES, apic::ioapic},
    mm::paddr_to_vaddr,
    trap::IrqLine,
};
static HPET_INSTANCE: Once<Hpet> = Once::new();

const OFFSET_ID_REGISTER: usize = 0x000;
const OFFSET_CONFIGURATION_REGISTER: usize = 0x010;
const OFFSET_INTERRUPT_STATUS_REGISTER: usize = 0x020;
const OFFSET_MAIN_COUNTER_VALUE_REGISTER: usize = 0x0F0;

const HPET_FREQ: usize = 1_000_000_000_000_000;

#[derive(Debug)]
#[repr(C)]
struct HpetTimerRegister {
    configuration_and_capabilities_register: u32,
    timer_compartor_value_register: u32,
    fsb_interrupt_route_register: u32,
}

struct Hpet {
    information_register: Volatile<&'static u32, ReadOnly>,
    general_configuration_register: Volatile<&'static mut u32, ReadWrite>,
    general_interrupt_status_register: Volatile<&'static mut u32, ReadWrite>,

    timer_registers: Vec<Volatile<&'static mut HpetTimerRegister, ReadWrite>>,
    irq: IrqLine,
}

impl Hpet {
    fn new(base_address: usize) -> Hpet {
        let information_register_ref = unsafe {
            &*(paddr_to_vaddr(base_address + OFFSET_ID_REGISTER) as *mut usize as *mut u32)
        };
        let general_configuration_register_ref = unsafe {
            &mut *(paddr_to_vaddr(base_address + OFFSET_CONFIGURATION_REGISTER) as *mut usize
                as *mut u32)
        };
        let general_interrupt_status_register_ref = unsafe {
            &mut *(paddr_to_vaddr(base_address + OFFSET_INTERRUPT_STATUS_REGISTER) as *mut usize
                as *mut u32)
        };

        let information_register = Volatile::new_read_only(information_register_ref);
        let general_configuration_register = Volatile::new(general_configuration_register_ref);
        let general_interrupt_status_register =
            Volatile::new(general_interrupt_status_register_ref);

        let num_comparator = ((information_register.read() & 0x1F00) >> 8) as u8 + 1;

        let mut comparators = Vec::with_capacity(num_comparator as usize);

        // Ensure that the addresses in the loop will not overflow
        base_address
            .checked_add(0x100 + num_comparator as usize * 0x20)
            .unwrap();
        for i in 0..num_comparator {
            let comp = Volatile::new(unsafe {
                &mut *(paddr_to_vaddr(base_address + 0x100 + i as usize * 0x20) as *mut usize
                    as *mut HpetTimerRegister)
            });
            comparators.push(comp);
        }

        let mut lock = ioapic::IO_APIC.get().unwrap()[0].lock();
        let irq = IrqLine::alloc().unwrap();
        // FIXME: The index of HPET interrupt needs to be tested.
        lock.enable(0, irq.clone()).unwrap();
        drop(lock);

        Hpet {
            information_register,
            general_configuration_register,
            general_interrupt_status_register,
            timer_registers: comparators,
            irq,
        }
    }

    pub fn hardware_rev(&self) -> u8 {
        (self.information_register.read() & 0xFF) as u8
    }

    pub fn num_comparators(&self) -> u8 {
        ((self.information_register.read() & 0x1F00) >> 8) as u8 + 1
    }

    pub fn main_counter_is_64bits(&self) -> bool {
        (self.information_register.read() & 0x2000) != 0
    }

    pub fn legacy_irq_capable(&self) -> bool {
        (self.information_register.read() & 0x8000) != 0
    }

    pub fn pci_vendor_id(&self) -> u16 {
        ((self.information_register.read() & 0xFFFF_0000) >> 16) as u16
    }
}

/// HPET init, need to init IOAPIC before init this function
pub fn init() -> Result<(), AcpiError> {
    let lock = ACPI_TABLES.get().unwrap().lock();

    let hpet_info = HpetInfo::new(&*lock)?;

    // config IO APIC entry
    let hpet = Hpet::new(hpet_info.base_address);
    HPET_INSTANCE.call_once(|| hpet);
    Ok(())
}
