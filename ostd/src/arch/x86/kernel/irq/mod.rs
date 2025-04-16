// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};
use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use ioapic::IoApic;
use log::info;
use spin::Once;

use super::acpi::get_acpi_tables;
use crate::{io::IoMemAllocatorBuilder, sync::SpinLock, trap::irq::IrqLine, Error, Result};

mod ioapic;
mod pic;

/// An IRQ chip.
///
/// This abstracts the hardware IRQ chips (or IRQ controllers), allowing the bus or device drivers
/// to enable [`IrqLine`]s (via, e.g., [`map_gsi_pin_to`]) regardless of the specifics of the IRQ chip.
///
/// In the x86 architecture, the underlying hardware is typically either 8259 Programmable
/// Interrupt Controller (PIC) or I/O Advanced Programmable Interrupt Controller (I/O APIC).
///
/// [`map_gsi_pin_to`]: Self::map_gsi_pin_to
pub struct IrqChip {
    io_apics: SpinLock<Box<[IoApic]>>,
    overrides: Box<[IsaOverride]>,
}

struct IsaOverride {
    /// ISA IRQ source.
    source: u8,
    /// GSI target.
    target: u32,
}

impl IrqChip {
    /// Maps an IRQ pin specified by a GSI number to an IRQ line.
    ///
    /// ACPI represents all interrupts as "flat" values known as global system interrupts (GSIs).
    /// So GSI numbers are well defined on all systems where the ACPI support is present.
    //
    // TODO: Confirm whether the interrupt numbers in the device tree on non-ACPI systems are the
    // same as the GSI numbers.
    pub fn map_gsi_pin_to(
        &'static self,
        irq_line: IrqLine,
        gsi_index: u32,
    ) -> Result<MappedIrqLine> {
        let mut io_apics = self.io_apics.lock();

        let io_apic = io_apics
            .iter_mut()
            .rev()
            .find(|io_apic| io_apic.interrupt_base() <= gsi_index)
            .unwrap();
        let index_in_io_apic = (gsi_index - io_apic.interrupt_base())
            .try_into()
            .map_err(|_| Error::InvalidArgs)?;
        io_apic.enable(index_in_io_apic, &irq_line)?;

        Ok(MappedIrqLine {
            irq_line,
            gsi_index,
            irq_chip: self,
        })
    }

    fn disable_gsi(&self, gsi_index: u32) {
        let mut io_apics = self.io_apics.lock();

        let io_apic = io_apics
            .iter_mut()
            .rev()
            .find(|io_apic| io_apic.interrupt_base() <= gsi_index)
            .unwrap();
        let index_in_io_apic = (gsi_index - io_apic.interrupt_base()) as u8;
        io_apic.disable(index_in_io_apic).unwrap();
    }

    /// Maps an IRQ pin specified by an ISA interrupt number to an IRQ line.
    ///
    /// Industry Standard Architecture (ISA) is the 16-bit internal bus of IBM PC/AT. For
    /// compatibility reasons, legacy devices such as keyboards connected via the i8042 PS/2
    /// controller still use it.
    ///
    /// This method is x86-specific.
    pub fn map_isa_pin_to(
        &'static self,
        irq_line: IrqLine,
        isa_index: u8,
    ) -> Result<MappedIrqLine> {
        let gsi_index = self
            .overrides
            .iter()
            .find(|isa_override| isa_override.source == isa_index)
            .map(|isa_override| isa_override.target)
            .unwrap_or(isa_index as u32);

        self.map_gsi_pin_to(irq_line, gsi_index)
    }

    /// Counts the number of I/O APICs.
    ///
    /// If I/O APICs are in use, this method counts how many I/O APICs are in use, otherwise, this
    /// method return zero.
    ///
    /// This method exists due to a workaround used in virtio-mmio bus probing. It should be
    /// removed once the workaround is retired. Therefore, only use this method if absolutely
    /// necessary.
    ///
    /// This method is x86-specific.
    pub fn count_io_apics(&self) -> usize {
        self.io_apics.lock().len()
    }
}

/// An [`IrqLine`] mapped to an IRQ pin managed by an [`IrqChip`].
///
/// When the object is dropped, the IRQ line will be unmapped by the IRQ chip.
pub struct MappedIrqLine {
    irq_line: IrqLine,
    gsi_index: u32,
    irq_chip: &'static IrqChip,
}

impl fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("gsi_index", &self.gsi_index)
            .finish_non_exhaustive()
    }
}

impl Deref for MappedIrqLine {
    type Target = IrqLine;

    fn deref(&self) -> &Self::Target {
        &self.irq_line
    }
}

impl DerefMut for MappedIrqLine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.irq_line
    }
}

impl Drop for MappedIrqLine {
    fn drop(&mut self) {
        self.irq_chip.disable_gsi(self.gsi_index)
    }
}

/// The [`IrqChip`] singleton.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

pub(in crate::arch) fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    use acpi::madt::{Madt, MadtEntry};

    // If there are no ACPI tables, or the ACPI tables do not provide us with information about
    // the I/O APIC, we may need to find another way to determine the I/O APIC address
    // correctly and reliably (e.g., by parsing the MultiProcessor Specification, which has
    // been deprecated for a long time and may not even exist in modern hardware).
    let acpi_tables = get_acpi_tables().unwrap();
    let madt_table = acpi_tables.find_table::<Madt>().unwrap();

    // "A one indicates that the system also has a PC-AT-compatible dual-8259 setup. The 8259
    // vectors must be disabled (that is, masked) when enabling the ACPI APIC operation"
    const PCAT_COMPAT: u32 = 1;
    if madt_table.get().flags & PCAT_COMPAT != 0 {
        pic::init_and_disable();
    }

    let mut io_apics = Vec::with_capacity(2);
    let mut isa_overrides = Vec::new();

    const BUS_ISA: u8 = 0; // "0 Constant, meaning ISA".

    for madt_entry in madt_table.get().entries() {
        match madt_entry {
            MadtEntry::IoApic(madt_io_apic) => {
                // SAFETY: We trust the ACPI tables (as well as the MADTs in them), from which the
                // base address is obtained, so it is a valid I/O APIC base address.
                let io_apic = unsafe {
                    IoApic::new(
                        madt_io_apic.io_apic_address as usize,
                        madt_io_apic.global_system_interrupt_base,
                        io_mem_builder,
                    )
                };
                io_apics.push(io_apic);
            }
            MadtEntry::InterruptSourceOverride(madt_isa_override)
                if madt_isa_override.bus == BUS_ISA =>
            {
                let isa_override = IsaOverride {
                    source: madt_isa_override.irq,
                    target: madt_isa_override.global_system_interrupt,
                };
                isa_overrides.push(isa_override);
            }
            _ => {}
        }
    }

    if isa_overrides.is_empty() {
        // TODO: QEMU MicroVM does not provide any interrupt source overrides. Therefore, the timer
        // interrupt used by the PIT will not work. Is this a bug in QEMU MicroVM? Why won't this
        // affect operating systems such as Linux?
        isa_overrides.push(IsaOverride {
            source: 0, // Timer ISA IRQ
            target: 2, // Timer GSI
        });
    }

    for isa_override in isa_overrides.iter() {
        info!(
            "[IOAPIC]: Override ISA interrupt {} for GSI {}",
            isa_override.source, isa_override.target
        );
    }

    io_apics.sort_by_key(|io_apic| io_apic.interrupt_base());
    assert!(!io_apics.is_empty(), "No I/O APICs found");
    assert_eq!(
        io_apics[0].interrupt_base(),
        0,
        "No I/O APIC with zero interrupt base found"
    );

    let irq_chip = IrqChip {
        io_apics: SpinLock::new(io_apics.into_boxed_slice()),
        overrides: isa_overrides.into_boxed_slice(),
    };
    IRQ_CHIP.call_once(|| irq_chip);
}
