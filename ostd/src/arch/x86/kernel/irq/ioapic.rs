// SPDX-License-Identifier: MPL-2.0

use core::ptr::NonNull;

use bit_field::BitField;
use cfg_if::cfg_if;
use log::info;
use volatile::{
    access::{ReadWrite, WriteOnly},
    VolatileRef,
};

use crate::{
    arch::if_tdx_enabled, io::IoMemAllocatorBuilder, mm::paddr_to_vaddr, trap::irq::IrqLine, Error,
    Result,
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        use crate::arch::tdx_guest;
    }
}

/// I/O Advanced Programmable Interrupt Controller (APIC).
///
/// It is used to distribute external interrupts in a more advanced manner than that of the
/// standard 8259 PIC. With the I/O APIC, interrupts can be distributed to physical or logical
/// (clusters of) processors and can be prioritized. Each I/O APIC typically handles 24 external
/// interrupts.
///
/// See also <https://wiki.osdev.org/IOAPIC>.
pub(super) struct IoApic {
    access: IoApicAccess,
    interrupt_base: u32,
}

impl IoApic {
    const TABLE_REG_BASE: u8 = 0x10;

    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid I/O APIC base address.
    pub(super) unsafe fn new(
        base_address: usize,
        base_interrupt: u32,
        io_mem_builder: &IoMemAllocatorBuilder,
    ) -> Self {
        let mut access = unsafe { IoApicAccess::new(base_address, io_mem_builder) };

        info!(
            "[IOAPIC]: Found at {:#x}, ID {}, version {}, interrupt base {}, interrupt count {}",
            base_address,
            access.id(),
            access.version(),
            base_interrupt,
            access.max_redirection_entry()
        );

        Self {
            access,
            interrupt_base: base_interrupt,
        }
    }

    /// Enables an entry.
    ///
    /// The caller should ensure that the IRQ line is not released before the entry is disabled.
    /// Otherwise, it will be considered a logical error.
    ///
    /// # Errors
    ///
    /// This method will fail if the index exceeds the I/O APIC's maximum redirection entry, or if
    /// the entry is in use.
    pub(super) fn enable(&mut self, index: u8, irq: &IrqLine) -> Result<()> {
        if index >= self.access.max_redirection_entry() {
            return Err(Error::InvalidArgs);
        }

        let value = self.access.read(Self::TABLE_REG_BASE + 2 * index);
        if value.get_bits(0..8) as u8 != 0 {
            return Err(Error::AccessDenied);
        }

        if let Some(remapping_index) = irq.remapping_index() {
            // Intel(R) Virtualization Technology for Directed I/O (Revision 5.0), Section 5.1.5.1
            // I/OxAPIC Programming says "Bit 48 in the I/OxAPIC RTE is Set to indicate the
            // Interrupt is in Remappable format."
            let mut value: u64 = irq.num() as u64 | 0x1_0000_0000_0000;

            // "The Interrupt_Index[14:0] is programmed in bits 63:49 of the I/OxAPIC RTE. The most
            // significant bit of the Interrupt_Index (Interrupt_Index[15]) is programmed in bit 11
            // of the I/OxAPIC RTE."
            value |= ((remapping_index & 0x8000) >> 4) as u64;
            value |= (remapping_index as u64 & 0x7FFF) << 49;

            self.access.write(
                Self::TABLE_REG_BASE + 2 * index,
                value.get_bits(0..32) as u32,
            );
            self.access.write(
                Self::TABLE_REG_BASE + 2 * index + 1,
                value.get_bits(32..64) as u32,
            );
        } else {
            self.access
                .write(Self::TABLE_REG_BASE + 2 * index, irq.num() as u32);
            self.access.write(Self::TABLE_REG_BASE + 2 * index + 1, 0);
        }

        Ok(())
    }

    /// Disables an entry.
    ///
    /// # Errors
    ///
    /// This method will fail if the index exceeds the I/O APIC's maximum redirection entry, or if
    /// the entry is not in use.
    pub(super) fn disable(&mut self, index: u8) -> Result<()> {
        if index >= self.access.max_redirection_entry() {
            return Err(Error::InvalidArgs);
        }

        // "Bit 16: Interrupt Mask - R/W. When this bit is 1, the interrupt signal is masked."
        self.access.write(Self::TABLE_REG_BASE + 2 * index, 1 << 16);
        self.access.write(Self::TABLE_REG_BASE + 2 * index + 1, 0);

        Ok(())
    }

    /// Returns the base number of the global system interrupts controlled by the I/O APIC.
    pub(super) fn interrupt_base(&self) -> u32 {
        self.interrupt_base
    }
}

struct IoApicAccess {
    register: VolatileRef<'static, u32, WriteOnly>,
    data: VolatileRef<'static, u32, ReadWrite>,
}

impl IoApicAccess {
    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid I/O APIC base address.
    pub(self) unsafe fn new(base_address: usize, io_mem_builder: &IoMemAllocatorBuilder) -> Self {
        io_mem_builder.remove(base_address..(base_address + 0x20));
        if_tdx_enabled!({
            assert_eq!(
                base_address % crate::mm::PAGE_SIZE,
                0,
                "[IOAPIC]: I/O memory is not page aligned, which cannot be unprotected in TDX: {:#x}",
                base_address,
            );
            // SAFETY:
            //  - The address range is page aligned, as we've checked above.
            //  - The caller guarantees that the address range represents the MMIO region for I/O
            //    APICs, so the address range must fall in the GPA limit.
            //  - FIXME: The I/O memory can be at a high address, so it may not be contained in the
            //    linear mapping.
            //  - Operations on the I/O memory can have side effects that may cause soundness
            //    problems, so the pages are not trivially untyped memory. However, since
            //    `io_mem_builder.remove()` ensures exclusive ownership, it's still fine to
            //    unprotect only once, before the I/O memory is used.
            unsafe { tdx_guest::unprotect_gpa_range(base_address, 1).unwrap() };
        });

        let register_addr = NonNull::new(paddr_to_vaddr(base_address) as *mut u32).unwrap();
        // SAFETY:
        // - The caller guarantees that the memory is an I/O APIC register.
        // - `io_mem_builder.remove()` guarantees that we have exclusive ownership of the register.
        let register = unsafe { VolatileRef::new_restricted(WriteOnly, register_addr) };

        let data_addr = NonNull::new(paddr_to_vaddr(base_address + 0x10) as *mut u32).unwrap();
        // SAFETY:
        // - The caller guarantees that the memory is an I/O APIC register.
        // - `io_mem_builder.remove()` guarantees that we have exclusive ownership of the register.
        let data = unsafe { VolatileRef::new(data_addr) };

        Self { register, data }
    }

    pub(self) fn read(&mut self, register: u8) -> u32 {
        self.register.as_mut_ptr().write(register as u32);
        self.data.as_ptr().read()
    }

    pub(self) fn write(&mut self, register: u8, data: u32) {
        self.register.as_mut_ptr().write(register as u32);
        self.data.as_mut_ptr().write(data);
    }

    pub(self) fn id(&mut self) -> u8 {
        self.read(0).get_bits(24..28) as u8
    }

    pub(self) fn version(&mut self) -> u8 {
        self.read(1).get_bits(0..9) as u8
    }

    pub(self) fn max_redirection_entry(&mut self) -> u8 {
        (self.read(1).get_bits(16..24) + 1) as u8
    }
}
