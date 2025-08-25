// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

mod plic;

use alloc::boxed::Box;
use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use spin::Once;

use crate::{
    arch::{
        boot::DEVICE_TREE,
        irq::{chip::plic::Plic, HwIrqLine, InterruptSource},
    },
    cpu::CpuId,
    io::IoMemAllocatorBuilder,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
    Result,
};

/// The [`IrqChip`] singleton.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

/// Initializes the Platform-Level Interrupt Controller (PLIC).
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once at a proper timing in the boot context.
/// 2. It is called before any other public functions of this module is called.
pub(in crate::arch) unsafe fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    let device_tree = DEVICE_TREE.get().unwrap();
    let mut plics = Plic::from_fdt(device_tree, io_mem_builder);
    plics.iter_mut().for_each(|plic| plic.init());
    IRQ_CHIP.call_once(|| IrqChip {
        plics: SpinLock::new(plics.into_boxed_slice()),
    });
    // SAFETY: Accessing the `sie` CSR to enable the external interrupt is safe
    // here because this function is only called during PLIC initialization,
    // and we ensure that only the external interrupt bit is set without
    // affecting other interrupt sources.
    unsafe { riscv::register::sie::set_sext() };
}

/// Initializes application-processor-specific PLIC state.
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once on this AP.
/// 2. It is called before any other public functions of this module is called
///    on this AP.
pub(in crate::arch) unsafe fn init_current_hart() {
    // SAFETY: Accessing the `sie` CSR to enable the external interrupt is safe
    // here due to the same reasons mentioned in `init`.
    unsafe { riscv::register::sie::set_sext() };
}

/// An IRQ chip.
///
/// This abstracts the hardware IRQ chips (or IRQ controllers), allowing the bus
/// or device drivers to enable [`IrqLine`]s (via, e.g., [`map_interrupt_source_to`])
/// regardless of the specifics of the IRQ chip.
///
/// In the RISC-V architecture, the underlying hardware is typically Platform-Level
/// Interrupt Controller (PLIC).
///
/// [`map_fdt_pin_to`]: Self::map_fdt_pin_to
pub struct IrqChip {
    plics: SpinLock<Box<[Plic]>, LocalIrqDisabled>,
}

impl IrqChip {
    /// Maps an IRQ pin specified by `interrupt_source_in_fdt` to an IRQ line.
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let mut plics = self.plics.lock();
        let (index, plic) = plics
            .iter_mut()
            .enumerate()
            .find(|(_, plic)| plic.phandle() == interrupt_source_in_fdt.interrupt_parent)
            .unwrap();

        plic.map_interrupt_source_to(interrupt_source_in_fdt.interrupt, &irq_line)?;
        plic.set_priority(interrupt_source_in_fdt.interrupt, 1);
        // FIXME: Here we only enable external insterrupt on the BSP. We should
        // enable it on APs as well when SMP is supported.
        plic.set_interrupt_enabled(CpuId::bsp().into(), interrupt_source_in_fdt.interrupt, true);

        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip: InterruptSourceOnChip {
                index,
                interrupt: interrupt_source_in_fdt.interrupt,
            },
        })
    }

    /// Claims an external interrupt that is pending on a specific hart.
    ///
    /// It returns the software IRQ number if there's a pending interrupt on the
    /// hart, otherwise it will return `None`.
    pub(in crate::arch) fn claim_interrupt(&self, hart: u32) -> Option<HwIrqLine> {
        self.plics
            .lock()
            .iter()
            .enumerate()
            .find_map(|(index, plic)| {
                let interrupt = plic.claim_interrupt(hart);
                plic.interrupt_number_mapping(interrupt)
                    .map(|irq_num| HwIrqLine {
                        irq_num,
                        source: InterruptSource::External(InterruptSourceOnChip {
                            index,
                            interrupt,
                        }),
                    })
            })
    }

    /// Acknowledges the completion of an interrupt.
    pub(super) fn complete_interrupt(
        &self,
        hart: u32,
        interrupt_source_on_chip: InterruptSourceOnChip,
    ) {
        let plics = self.plics.lock();
        plics[interrupt_source_on_chip.index]
            .complete_interrupt(hart, interrupt_source_on_chip.interrupt);
    }

    /// Unmaps an IRQ line from the IRQ chip.
    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        let mut plics = self.plics.lock();

        let InterruptSourceOnChip { index, interrupt } = &mapped_irq_line.interrupt_source_on_chip;
        let plic = &mut plics[*index];

        // FIXME: Here we only disable external insterrupt on the BSP. We should
        // disable it on APs as well when SMP is supported.
        plic.set_interrupt_enabled(CpuId::bsp().into(), *interrupt, false);
        plic.set_priority(*interrupt, 0);
        plic.unmap_interrupt_source(*interrupt);
    }
}

/// An [`IrqLine`] mapped to an IRQ pin managed by the [`IRQ_CHIP`].
///
/// When the object is dropped, the IRQ line will be unmapped by the IRQ chip.
pub struct MappedIrqLine {
    irq_line: IrqLine,
    interrupt_source_on_chip: InterruptSourceOnChip,
}

impl fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("interrupt_source_on_chip", &self.interrupt_source_on_chip)
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
        IRQ_CHIP.get().unwrap().unmap_irq_line(self)
    }
}

/// Interrupt source identifier in the device tree.
#[derive(Clone, Copy, Debug)]
pub struct InterruptSourceInFdt {
    /// Phandle of the interrupt controller it connects to.
    pub interrupt_parent: u32,
    /// Interrupt source number on the interrupt controller.
    pub interrupt: u32,
}

/// Interrupt source identifier on the `IRQ_CHIP`.
#[derive(Clone, Copy, Debug)]
pub(super) struct InterruptSourceOnChip {
    /// Index of the interrupt controller it connects to on `IRQ_CHIP`.
    index: usize,
    /// Interrupt source number on the interrupt controller.
    interrupt: u32,
}
