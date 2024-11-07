// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::{vec, vec::Vec};

use acpi::PlatformInfo;
use bit_field::BitField;
use cfg_if::cfg_if;
use log::info;
use spin::Once;

use crate::{
    arch::{iommu::has_interrupt_remapping, x86::kernel::acpi::ACPI_TABLES},
    mm::paddr_to_vaddr,
    sync::SpinLock,
    trap::IrqLine,
    Error, Result,
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        use ::tdx_guest::tdx_is_enabled;
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
        self.access.register.addr()
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
    register: *mut u32,
    data: *mut u32,
}

impl IoApicAccess {
    /// # Safety
    ///
    /// User must ensure the base address is valid.
    unsafe fn new(base_address: usize) -> Self {
        let vaddr = paddr_to_vaddr(base_address);
        Self {
            register: vaddr as *mut u32,
            data: (vaddr + 0x10) as *mut u32,
        }
    }

    pub fn read(&mut self, register: u8) -> u32 {
        // SAFETY: Since the base address is valid, the read/write should be safe.
        unsafe {
            self.register.write_volatile(register as u32);
            self.data.read_volatile()
        }
    }

    pub fn write(&mut self, register: u8, data: u32) {
        // SAFETY: Since the base address is valid, the read/write should be safe.
        unsafe {
            self.register.write_volatile(register as u32);
            self.data.write_volatile(data);
        }
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

/// # Safety: The pointer inside the IoApic will not change
unsafe impl Send for IoApic {}
/// # Safety: The pointer inside the IoApic will not change
unsafe impl Sync for IoApic {}

pub static IO_APIC: Once<Vec<SpinLock<IoApic>>> = Once::new();

pub fn init() {
    if !ACPI_TABLES.is_completed() {
        IO_APIC.call_once(|| {
            // FIXME: Is it possible to have an address that is not the default 0xFEC0_0000?
            // Need to find a way to determine if it is a valid address or not.
            const IO_APIC_DEFAULT_ADDRESS: usize = 0xFEC0_0000;
            #[cfg(feature = "cvm_guest")]
            // SAFETY:
            // This is safe because we are ensuring that the `IO_APIC_DEFAULT_ADDRESS` is a valid MMIO address before this operation.
            // The `IO_APIC_DEFAULT_ADDRESS` is a well-known address used for IO APICs in x86 systems.
            // We are also ensuring that we are only unprotecting a single page.
            if tdx_is_enabled() {
                unsafe {
                    tdx_guest::unprotect_gpa_range(IO_APIC_DEFAULT_ADDRESS, 1).unwrap();
                }
            }
            let mut io_apic = unsafe { IoApicAccess::new(IO_APIC_DEFAULT_ADDRESS) };
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
    }
    let table = ACPI_TABLES.get().unwrap().lock();
    let platform_info = PlatformInfo::new(&*table).unwrap();
    match platform_info.interrupt_model {
        acpi::InterruptModel::Unknown => panic!("not found APIC in ACPI Table"),
        acpi::InterruptModel::Apic(apic) => {
            let mut vec = Vec::new();
            for id in 0..apic.io_apics.len() {
                let io_apic = apic.io_apics.get(id).unwrap();
                #[cfg(feature = "cvm_guest")]
                // SAFETY:
                // This is safe because we are ensuring that the `io_apic.address` is a valid MMIO address before this operation.
                // We are also ensuring that we are only unprotecting a single page.
                if tdx_is_enabled() {
                    unsafe {
                        tdx_guest::unprotect_gpa_range(io_apic.address as usize, 1).unwrap();
                    }
                }
                let interrupt_base = io_apic.global_system_interrupt_base;
                let mut io_apic = unsafe { IoApicAccess::new(io_apic.address as usize) };
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
