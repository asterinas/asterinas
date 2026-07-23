// SPDX-License-Identifier: MPL-2.0

//! Interrupt controllers.

mod aplic;
mod imsic;
mod plic;

use alloc::{boxed::Box, vec::Vec};
use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use spin::Once;

use crate::{
    Error, Result,
    arch::{
        boot::DEVICE_TREE,
        irq::{
            HwIrqLine, InterruptSource,
            chip::{aplic::Aplic, imsic::Imsic, plic::Plic},
        },
    },
    io::IoMemAllocatorBuilder,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};

/// The [`IrqChip`] singleton.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

/// Initializes the platform interrupt controllers on the BSP.
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once at a proper timing in the boot context
///    of the BSP.
/// 2. It is called before any other public functions of this module is called.
pub(in crate::arch) unsafe fn init_on_bsp(io_mem_builder: &IoMemAllocatorBuilder) {
    let device_tree = DEVICE_TREE.get().unwrap();
    let mut plics = Plic::from_fdt(device_tree, io_mem_builder);
    plics.iter_mut().for_each(Plic::init);

    let imsic = Imsic::from_fdt(device_tree, io_mem_builder);
    let mut aplics = imsic.as_ref().map_or_else(Vec::new, |imsic| {
        Aplic::from_fdt(device_tree, io_mem_builder, imsic.phandle())
    });
    aplics.iter_mut().for_each(Aplic::init);

    IRQ_CHIP.call_once(|| IrqChip {
        plics: SpinLock::new(plics.into_boxed_slice()),
        aplics: SpinLock::new(aplics.into_boxed_slice()),
        imsic: imsic.map(SpinLock::new),
    });
    IRQ_CHIP.get().unwrap().init_current_hart();

    // SAFETY: The interrupt controllers for this hart are initialized before
    // enabling supervisor external interrupts.
    unsafe { riscv::register::sie::set_sext() };
}

/// Initializes application-processor-specific interrupt controller state.
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once on this AP.
/// 2. It is called before any other public functions of this module is called
///    on this AP.
pub(in crate::arch) unsafe fn init_on_ap() {
    IRQ_CHIP.get().unwrap().init_current_hart();
    // SAFETY: The interrupt controllers for this hart are initialized before
    // enabling supervisor external interrupts.
    unsafe { riscv::register::sie::set_sext() };
}

/// The platform external interrupt controllers.
pub struct IrqChip {
    plics: SpinLock<Box<[Plic]>, LocalIrqDisabled>,
    aplics: SpinLock<Box<[Aplic]>, LocalIrqDisabled>,
    imsic: Option<SpinLock<Imsic, LocalIrqDisabled>>,
}

impl IrqChip {
    /// Maps an interrupt source described by the device tree to an IRQ line.
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let mut plics = self.plics.lock();
        if let Some((index, plic)) = plics
            .iter_mut()
            .enumerate()
            .find(|(_, plic)| plic.phandle() == interrupt_source_in_fdt.interrupt_parent)
        {
            plic.map_interrupt_source_to(interrupt_source_in_fdt.interrupt, &irq_line)?;
            plic.set_priority(interrupt_source_in_fdt.interrupt, 1);
            plic.managed_harts().for_each(|hart| {
                plic.set_interrupt_enabled(hart, interrupt_source_in_fdt.interrupt, true)
            });
            return Ok(MappedIrqLine {
                irq_line,
                interrupt_source_on_chip: InterruptSourceOnChip {
                    controller: InterruptController::Plic,
                    index,
                    interrupt: interrupt_source_in_fdt.interrupt,
                },
            });
        }
        drop(plics);

        let mut aplics = self.aplics.lock();
        let Some((index, aplic)) = aplics
            .iter_mut()
            .enumerate()
            .find(|(_, aplic)| aplic.phandle() == interrupt_source_in_fdt.interrupt_parent)
        else {
            return Err(Error::InvalidArgs);
        };
        let msi_interrupt_id = Self::msi_interrupt_id(irq_line.num());
        self.enable_msi(irq_line.num())?;
        if let Err(err) = aplic.map_interrupt_source_to(
            interrupt_source_in_fdt.interrupt,
            interrupt_source_in_fdt.trigger,
            &irq_line,
            msi_interrupt_id,
        ) {
            self.disable_msi(irq_line.num());
            return Err(err);
        }

        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip: InterruptSourceOnChip {
                controller: InterruptController::Aplic,
                index,
                interrupt: interrupt_source_in_fdt.interrupt,
            },
        })
    }

    /// Claims an external interrupt pending on the current hart.
    pub(in crate::arch) fn claim_interrupt(&self, hart: u32) -> Option<HwIrqLine> {
        if let Some(imsic) = &self.imsic
            && let Some(msi_interrupt_id) = imsic.lock().claim()
            && let Some(irq_num) = Self::irq_num_from_msi_interrupt_id(msi_interrupt_id)
        {
            return Some(HwIrqLine {
                irq_num,
                source: InterruptSource::Message,
            });
        }

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
                            controller: InterruptController::Plic,
                            index,
                            interrupt,
                        }),
                    })
            })
    }

    /// Returns the MSI write address for the currently supported supervisor
    /// IMSIC target.
    ///
    /// The initial implementation uses the first supervisor IMSIC MMIO region
    /// discovered from the device tree. Multi-hart target selection should pass
    /// an explicit hart in the future.
    pub fn msi_address(&self) -> Option<usize> {
        self.imsic
            .as_ref()
            .map(|imsic| imsic.lock().message_address())
    }

    /// Enables the software IRQ's MSI identity in the current hart's IMSIC
    /// interrupt file.
    pub fn enable_msi(&self, irq_num: u8) -> Result<()> {
        let Some(imsic) = &self.imsic else {
            return Err(Error::NotEnoughResources);
        };
        if imsic.lock().enable(Self::msi_interrupt_id(irq_num)) {
            Ok(())
        } else {
            Err(Error::InvalidArgs)
        }
    }

    fn disable_msi(&self, irq_num: u8) {
        if let Some(imsic) = &self.imsic {
            imsic.lock().disable(Self::msi_interrupt_id(irq_num));
        }
    }

    fn msi_interrupt_id(irq_num: u8) -> u16 {
        u16::from(irq_num) + 1
    }

    fn irq_num_from_msi_interrupt_id(interrupt_id: u16) -> Option<u8> {
        interrupt_id
            .checked_sub(1)
            .and_then(|irq_num| u8::try_from(irq_num).ok())
    }

    fn init_current_hart(&self) {
        if let Some(imsic) = &self.imsic {
            imsic.lock().init_current_hart();
        }
    }

    pub(super) fn complete_interrupt(
        &self,
        hart: u32,
        interrupt_source_on_chip: InterruptSourceOnChip,
    ) {
        if interrupt_source_on_chip.controller != InterruptController::Plic {
            return;
        }
        let plics = self.plics.lock();
        plics[interrupt_source_on_chip.index]
            .complete_interrupt(hart, interrupt_source_on_chip.interrupt);
    }

    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        let source = mapped_irq_line.interrupt_source_on_chip;
        match source.controller {
            InterruptController::Plic => {
                let mut plics = self.plics.lock();
                let plic = &mut plics[source.index];
                plic.managed_harts()
                    .for_each(|hart| plic.set_interrupt_enabled(hart, source.interrupt, false));
                plic.set_priority(source.interrupt, 0);
                plic.unmap_interrupt_source(source.interrupt);
            }
            InterruptController::Aplic => {
                self.aplics.lock()[source.index].unmap_interrupt_source(source.interrupt);
                self.disable_msi(mapped_irq_line.irq_line.num());
            }
        }
    }
}

/// An [`IrqLine`] mapped to an interrupt-controller source.
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
    /// Electrical trigger mode encoded by the device tree.
    pub trigger: InterruptTrigger,
}

impl InterruptSourceInFdt {
    /// Decodes an interrupt specifier returned by `FdtNode::interrupts`.
    pub fn new(interrupt_parent: u32, specifier: usize) -> Self {
        let specifier = specifier as u64;
        let (interrupt, flags) = if specifier > u32::MAX as u64 {
            ((specifier >> 32) as u32, specifier as u32)
        } else {
            (specifier as u32, 0)
        };
        Self {
            interrupt_parent,
            interrupt,
            trigger: InterruptTrigger::from_fdt_flags(flags),
        }
    }
}

/// Interrupt source trigger mode.
#[derive(Clone, Copy, Debug)]
pub enum InterruptTrigger {
    EdgeRising,
    EdgeFalling,
    LevelHigh,
    LevelLow,
}

impl InterruptTrigger {
    fn from_fdt_flags(flags: u32) -> Self {
        match flags & 0xf {
            1 => Self::EdgeRising,
            2 => Self::EdgeFalling,
            8 => Self::LevelLow,
            _ => Self::LevelHigh,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterruptController {
    Plic,
    Aplic,
}

/// Interrupt source identifier on the platform interrupt controller.
#[derive(Clone, Copy, Debug)]
pub(super) struct InterruptSourceOnChip {
    controller: InterruptController,
    index: usize,
    interrupt: u32,
}
