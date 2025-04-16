// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::ptr::NonNull;

use acpi::{AcpiError, HpetInfo};
use spin::Once;
use volatile::{
    access::{ReadOnly, ReadWrite},
    VolatileRef,
};

use crate::{
    arch::kernel::{acpi::get_acpi_tables, MappedIrqLine, IRQ_CHIP},
    mm::paddr_to_vaddr,
    trap::irq::IrqLine,
};

static HPET_INSTANCE: Once<Hpet> = Once::new();

const OFFSET_ID_REGISTER: usize = 0x000;
const OFFSET_CONFIGURATION_REGISTER: usize = 0x010;
const OFFSET_INTERRUPT_STATUS_REGISTER: usize = 0x020;
#[expect(dead_code)]
const OFFSET_MAIN_COUNTER_VALUE_REGISTER: usize = 0x0F0;

#[expect(dead_code)]
const HPET_FREQ: usize = 1_000_000_000_000_000;

#[derive(Debug)]
#[repr(C)]
struct HpetTimerRegister {
    configuration_and_capabilities_register: u32,
    timer_comparator_value_register: u32,
    fsb_interrupt_route_register: u32,
}

struct Hpet {
    information_register: VolatileRef<'static, u32, ReadOnly>,
    _general_configuration_register: VolatileRef<'static, u32, ReadWrite>,
    _general_interrupt_status_register: VolatileRef<'static, u32, ReadWrite>,

    _timer_registers: Vec<VolatileRef<'static, HpetTimerRegister, ReadWrite>>,
    _irq: MappedIrqLine,
}

impl Hpet {
    /// # Safety
    ///
    /// The caller must ensure that the address is valid and points to the HPET MMIO region.
    unsafe fn new(base_address: NonNull<u8>) -> Hpet {
        // SAFETY: The safety is upheld by the caller.
        let (
            information_register,
            general_configuration_register,
            general_interrupt_status_register,
        ) = unsafe {
            (
                VolatileRef::new_read_only(base_address.add(OFFSET_ID_REGISTER).cast::<u32>()),
                VolatileRef::new(
                    base_address
                        .add(OFFSET_CONFIGURATION_REGISTER)
                        .cast::<u32>(),
                ),
                VolatileRef::new(
                    base_address
                        .add(OFFSET_INTERRUPT_STATUS_REGISTER)
                        .cast::<u32>(),
                ),
            )
        };

        let num_comparator = ((information_register.as_ptr().read() & 0x1F00) >> 8) as u8 + 1;
        let num_comparator = num_comparator as usize;

        // FIXME: We now trust the hardware. We should instead find a way to check that
        // `num_comparator` are reasonable values before proceeding.

        let mut comparators = Vec::with_capacity(num_comparator);
        for i in 0..num_comparator {
            // SAFETY: The safety is upheld by the caller and the correctness of the information
            // value.
            let comp = unsafe {
                VolatileRef::new(
                    base_address
                        .add(0x100)
                        .add(i * 0x20)
                        .cast::<HpetTimerRegister>(),
                )
            };
            comparators.push(comp);
        }

        let irq = IrqLine::alloc().unwrap();
        // FIXME: The index of HPET interrupt needs to be tested.
        let irq = IRQ_CHIP.get().unwrap().map_isa_pin_to(irq, 0).unwrap();

        Hpet {
            information_register,
            _general_configuration_register: general_configuration_register,
            _general_interrupt_status_register: general_interrupt_status_register,
            _timer_registers: comparators,
            _irq: irq,
        }
    }

    #[expect(dead_code)]
    pub fn hardware_rev(&self) -> u8 {
        (self.information_register.as_ptr().read() & 0xFF) as u8
    }

    #[expect(dead_code)]
    pub fn num_comparators(&self) -> u8 {
        ((self.information_register.as_ptr().read() & 0x1F00) >> 8) as u8 + 1
    }

    #[expect(dead_code)]
    pub fn main_counter_is_64bits(&self) -> bool {
        (self.information_register.as_ptr().read() & 0x2000) != 0
    }

    #[expect(dead_code)]
    pub fn legacy_irq_capable(&self) -> bool {
        (self.information_register.as_ptr().read() & 0x8000) != 0
    }

    #[expect(dead_code)]
    pub fn pci_vendor_id(&self) -> u16 {
        ((self.information_register.as_ptr().read() & 0xFFFF_0000) >> 16) as u16
    }
}

/// HPET init, need to init IOAPIC before init this function
#[expect(dead_code)]
pub fn init() -> Result<(), AcpiError> {
    let tables = get_acpi_tables().unwrap();

    let hpet_info = HpetInfo::new(&tables)?;
    assert_ne!(hpet_info.base_address, 0, "HPET address should not be zero");

    let base = NonNull::new(paddr_to_vaddr(hpet_info.base_address) as *mut u8).unwrap();
    // SAFETY: The base address is from the ACPI table and points to the HPET MMIO region.
    let hpet = unsafe { Hpet::new(base) };
    HPET_INSTANCE.call_once(|| hpet);

    Ok(())
}
