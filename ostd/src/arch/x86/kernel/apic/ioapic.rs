// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use alloc::{vec, vec::Vec};
use core::ptr::NonNull;

use bit_field::BitField;
use cfg_if::cfg_if;
use log::info;
use spin::Once;
use volatile::{
    access::{ReadWrite, WriteOnly},
    VolatileRef,
};

use crate::{
    arch::{if_tdx_enabled, iommu::has_interrupt_remapping, kernel::acpi::get_platform_info},
    io::IoMemAllocatorBuilder,
    mm::paddr_to_vaddr,
    sync::SpinLock,
    trap::IrqLine,
    Error, Result,
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        use crate::arch::tdx_guest;
    }
}

/// I/O Advanced Programmable Interrupt Controller. It is used to distribute external interrupts
/// in a more advanced manner than that of the standard 8259 PIC.
///
/// User can enable external interrupts by specifying IRQ and the external interrupt line index,
/// such as the terminal input being interrupt line 0.
///
/// Ref: https://wiki.osdev.org/IOAPIC
pub struct IoApic {
    access: IoApicAccess,
    irqs: Vec<IrqLine>,
    interrupt_base: u32,
}

impl IoApic {
    const TABLE_REG_BASE: u8 = 0x10;

    /// Enables an entry. The index should not exceed the `max_redirection_entry`
    pub fn enable(&mut self, index: u8, irq: IrqLine) -> Result<()> {
        if index >= self.max_redirection_entry() {
            return Err(Error::InvalidArgs);
        }
        let value = self.access.read(Self::TABLE_REG_BASE + 2 * index);
        if value.get_bits(0..8) as u8 != 0 {
            return Err(Error::AccessDenied);
        }
        if has_interrupt_remapping() {
            let mut handle = irq.inner_irq().bind_remapping_entry().unwrap().lock();

            // Enable irt entry
            let irt_entry_mut = handle.irt_entry_mut().unwrap();
            irt_entry_mut.enable_default(irq.num() as u32);

            // Construct remappable format RTE with RTE[48] set.
            let mut value: u64 = irq.num() as u64 | 0x1_0000_0000_0000;

            // Interrupt index[14:0] is on RTE[63:49] and interrupt index[15] is on RTE[11].
            value |= ((handle.index() & 0x8000) >> 4) as u64;
            value |= (handle.index() as u64 & 0x7FFF) << 49;

            self.access.write(
                Self::TABLE_REG_BASE + 2 * index,
                value.get_bits(0..32) as u32,
            );
            self.access.write(
                Self::TABLE_REG_BASE + 2 * index + 1,
                value.get_bits(32..64) as u32,
            );

            drop(handle);
            self.irqs.push(irq);
            return Ok(());
        }

        self.access
            .write(Self::TABLE_REG_BASE + 2 * index, irq.num() as u32);
        self.access.write(Self::TABLE_REG_BASE + 2 * index + 1, 0);
        self.irqs.push(irq);
        Ok(())
    }

    /// Disables an entry. The index should not exceed the `max_redirection_entry`
    pub fn disable(&mut self, index: u8) -> Result<()> {
        if index >= self.max_redirection_entry() {
            return Err(Error::InvalidArgs);
        }
        let value = self.access.read(Self::TABLE_REG_BASE + 2 * index);
        let irq_num = value.get_bits(0..8) as u8;
        // mask interrupt
        self.access.write(Self::TABLE_REG_BASE + 2 * index, 1 << 16);
        self.access.write(Self::TABLE_REG_BASE + 2 * index + 1, 0);
        self.irqs.retain(|h| h.num() != irq_num);
        Ok(())
    }

    /// The global system interrupt number where this I/O APIC's inputs start, typically 0.
    pub fn interrupt_base(&self) -> u32 {
        self.interrupt_base
    }

    pub fn id(&mut self) -> u8 {
        self.access.id()
    }

    pub fn version(&mut self) -> u8 {
        self.access.version()
    }

    pub fn max_redirection_entry(&mut self) -> u8 {
        self.access.max_redirection_entry()
    }

    pub fn vaddr(&self) -> usize {
        self.access.register.as_ptr().as_raw_ptr().addr().get()
    }

    fn new(io_apic_access: IoApicAccess, interrupt_base: u32) -> Self {
        Self {
            access: io_apic_access,
            irqs: Vec::new(),
            interrupt_base,
        }
    }
}

struct IoApicAccess {
    register: VolatileRef<'static, u32, WriteOnly>,
    data: VolatileRef<'static, u32, ReadWrite>,
}

impl IoApicAccess {
    /// # Safety
    ///
    /// User must ensure the base address is valid.
    unsafe fn new(base_address: usize, io_mem_builder: &IoMemAllocatorBuilder) -> Self {
        io_mem_builder.remove(base_address..(base_address + 0x20));
        let base = NonNull::new(paddr_to_vaddr(base_address) as *mut u8).unwrap();
        let register = VolatileRef::new_restricted(WriteOnly, base.cast::<u32>());
        let data = VolatileRef::new(base.add(0x10).cast::<u32>());
        Self { register, data }
    }

    pub fn read(&mut self, register: u8) -> u32 {
        self.register.as_mut_ptr().write(register as u32);
        self.data.as_ptr().read()
    }

    pub fn write(&mut self, register: u8, data: u32) {
        self.register.as_mut_ptr().write(register as u32);
        self.data.as_mut_ptr().write(data);
    }

    pub fn id(&mut self) -> u8 {
        self.read(0).get_bits(24..28) as u8
    }

    pub fn set_id(&mut self, id: u8) {
        self.write(0, (id as u32) << 24)
    }

    pub fn version(&mut self) -> u8 {
        self.read(1).get_bits(0..9) as u8
    }

    pub fn max_redirection_entry(&mut self) -> u8 {
        (self.read(1).get_bits(16..24) + 1) as u8
    }
}

pub static IO_APIC: Once<Vec<SpinLock<IoApic>>> = Once::new();

pub fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    let Some(platform_info) = get_platform_info() else {
        IO_APIC.call_once(|| {
            // FIXME: Is it possible to have an address that is not the default 0xFEC0_0000?
            // Need to find a way to determine if it is a valid address or not.
            const IO_APIC_DEFAULT_ADDRESS: usize = 0xFEC0_0000;
            if_tdx_enabled!({
                // SAFETY:
                // This is safe because we are ensuring that the `IO_APIC_DEFAULT_ADDRESS` is a valid MMIO address before this operation.
                // The `IO_APIC_DEFAULT_ADDRESS` is a well-known address used for IO APICs in x86 systems.
                // We are also ensuring that we are only unprotecting a single page.
                unsafe {
                    tdx_guest::unprotect_gpa_range(IO_APIC_DEFAULT_ADDRESS, 1).unwrap();
                }
            });
            let mut io_apic = unsafe { IoApicAccess::new(IO_APIC_DEFAULT_ADDRESS, io_mem_builder) };
            io_apic.set_id(0);
            let id = io_apic.id();
            let version = io_apic.version();
            let max_redirection_entry = io_apic.max_redirection_entry();
            info!(
                "[IOAPIC]: Not found ACPI tables, using default address:{:x?}",
                IO_APIC_DEFAULT_ADDRESS,
            );
            info!(
                "[IOAPIC]: IOAPIC id: {}, version:{}, max_redirection_entry:{}, interrupt base:{}",
                id, version, max_redirection_entry, 0
            );
            vec![SpinLock::new(IoApic::new(io_apic, 0))]
        });
        return;
    };
    match &platform_info.interrupt_model {
        acpi::InterruptModel::Unknown => panic!("not found APIC in ACPI Table"),
        acpi::InterruptModel::Apic(apic) => {
            let mut vec = Vec::new();
            for id in 0..apic.io_apics.len() {
                let io_apic = apic.io_apics.get(id).unwrap();
                if_tdx_enabled!({
                    // SAFETY:
                    // This is safe because we are ensuring that the `io_apic.address` is a valid MMIO address before this operation.
                    // We are also ensuring that we are only unprotecting a single page.
                    unsafe {
                        tdx_guest::unprotect_gpa_range(io_apic.address as usize, 1).unwrap();
                    }
                });
                let interrupt_base = io_apic.global_system_interrupt_base;
                let mut io_apic =
                    unsafe { IoApicAccess::new(io_apic.address as usize, io_mem_builder) };
                io_apic.set_id(id as u8);
                let id = io_apic.id();
                let version = io_apic.version();
                let max_redirection_entry = io_apic.max_redirection_entry();
                info!(
                    "[IOAPIC]: IOAPIC id: {}, version:{}, max_redirection_entry:{}, interrupt base:{}",
                    id, version, max_redirection_entry, interrupt_base
                );
                vec.push(SpinLock::new(IoApic::new(io_apic, interrupt_base)));
            }
            if vec.is_empty() {
                panic!("[IOAPIC]: Not exists IOAPIC");
            }
            IO_APIC.call_once(|| vec);
        }
        _ => {
            panic!("Unknown interrupt model")
        }
    };
}
