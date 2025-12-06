// SPDX-License-Identifier: MPL-2.0

use bit_field::BitField;
use log::info;

use crate::{
    Error, Result,
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
};

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
    max_redirection_entry: u8,
}

impl IoApic {
    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid I/O APIC base address.
    pub(super) unsafe fn new(
        base_address: usize,
        base_interrupt: u32,
        io_mem_builder: &IoMemAllocatorBuilder,
    ) -> Self {
        let mut access = unsafe { IoApicAccess::new(base_address, io_mem_builder) };
        let max_redirection_entry = access.max_redirection_entry();

        info!(
            "[IOAPIC]: Found at {:#x}, ID {}, version {}, interrupt base {}, interrupt count {}",
            base_address,
            access.id(),
            access.version(),
            base_interrupt,
            max_redirection_entry,
        );

        let mut ioapic = Self {
            access,
            interrupt_base: base_interrupt,
            max_redirection_entry,
        };

        // Initialize all the entries to the disabled state.
        for index in 0..=max_redirection_entry {
            ioapic.disable(index).unwrap();
        }

        ioapic
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
        if index > self.max_redirection_entry {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: `index` is inbound. The redirection table is safe to read.
        let value = unsafe { self.access.read(IoApicAccess::IOREDTBL + 2 * index) };
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

            // SAFETY: `index` is inbound. It is safe to enable the redirection entry with the
            // correct remapping index.
            unsafe {
                self.access.write(
                    IoApicAccess::IOREDTBL + 2 * index,
                    value.get_bits(0..32) as u32,
                );
                self.access.write(
                    IoApicAccess::IOREDTBL + 2 * index + 1,
                    value.get_bits(32..64) as u32,
                );
            }
        } else {
            // SAFETY: `index` is inbound. It is safe to enable the redirection entry with the
            // legal IRQ number.
            unsafe {
                self.access
                    .write(IoApicAccess::IOREDTBL + 2 * index, irq.num() as u32);
                self.access.write(IoApicAccess::IOREDTBL + 2 * index + 1, 0);
            }
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
        if index > self.max_redirection_entry {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: `index` is inbound. Disabling the redirection entry is always safe.
        unsafe {
            // "Bit 16: Interrupt Mask - R/W. When this bit is 1, the interrupt signal is masked."
            self.access
                .write(IoApicAccess::IOREDTBL + 2 * index, 1 << 16);
            self.access.write(IoApicAccess::IOREDTBL + 2 * index + 1, 0);
        }

        Ok(())
    }

    /// Returns the base number of the global system interrupts controlled by the I/O APIC.
    pub(super) fn interrupt_base(&self) -> u32 {
        self.interrupt_base
    }
}

struct IoApicAccess {
    io_mem: IoMem<Sensitive>,
}

impl IoApicAccess {
    /// I/O Register Select (index).
    const MMIO_REGSEL: usize = 0x00;
    /// I/O Window (data).
    const MMIO_WIN: usize = 0x10;
    /// The size of the MMIO region.
    ///
    /// I/O APICs only have two MMIO registers, at offsets 0x00 and 0x10. Therefore, the size of
    /// the MMIO region may be 0x20. However, we use a page here because (1) multiple I/O APICs
    /// typically use different MMIO pages and (2) TD guests do not support sub-page MMIO regions.
    const MMIO_SIZE: usize = crate::mm::PAGE_SIZE;

    /// IOAPIC ID.
    const IOAPICID: u8 = 0x00;
    /// IOAPIC Version.
    const IOAPICVER: u8 = 0x01;
    /// Redirection Table.
    pub(self) const IOREDTBL: u8 = 0x10;

    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid I/O APIC base address.
    pub(self) unsafe fn new(base_address: usize, io_mem_builder: &IoMemAllocatorBuilder) -> Self {
        let io_mem = io_mem_builder.reserve(
            base_address..(base_address + Self::MMIO_SIZE),
            crate::mm::CachePolicy::Uncacheable,
        );

        Self { io_mem }
    }

    pub(self) unsafe fn read(&mut self, register: u8) -> u32 {
        // SAFETY: This reads data from an I/O APIC register. The safety is upheld by the caller.
        unsafe {
            self.io_mem
                .write_once(Self::MMIO_REGSEL, &(register as u32));
            self.io_mem.read_once(Self::MMIO_WIN)
        }
    }

    pub(self) unsafe fn write(&mut self, register: u8, data: u32) {
        // SAFETY: This writes data to an I/O APIC register. The safety is upheld by the caller.
        unsafe {
            self.io_mem
                .write_once(Self::MMIO_REGSEL, &(register as u32));
            self.io_mem.write_once(Self::MMIO_WIN, &data);
        }
    }

    pub(self) fn id(&mut self) -> u8 {
        // IOAPICID: "Bit 24-27: IOAPIC Identification - R/W."
        // SAFETY: IOAPICID is safe to read.
        unsafe { self.read(Self::IOAPICID).get_bits(24..28) as u8 }
    }

    pub(self) fn version(&mut self) -> u8 {
        // IOAPICVER: "Bit 7-0: APIC VERSION - RO."
        // SAFETY: IOAPICVER is safe to read.
        unsafe { self.read(Self::IOAPICVER).get_bits(0..8) as u8 }
    }

    pub(self) fn max_redirection_entry(&mut self) -> u8 {
        // IOAPICVER: "Bit 16-23: Maximum Redirection Entry - RO."
        // SAFETY: IOAPICVER is safe to read.
        unsafe { self.read(Self::IOAPICVER).get_bits(16..24) as u8 }
    }
}
