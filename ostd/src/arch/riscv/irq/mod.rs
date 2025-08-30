// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

mod plic;

use alloc::{boxed::Box, vec::Vec};
use core::{
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::Ordering,
};

use spin::Once;

use crate::{
    arch::{boot::DEVICE_TREE, irq::plic::Plic},
    cpu::{CpuId, PinCurrentCpu},
    io::IoMemAllocatorBuilder,
    mm::kspace::IS_KERNEL_PAGE_TABLE_ACTIVATED,
    sync::SpinLock,
    trap::irq::IrqLine,
    Result,
};

// FIXME: Remove these two x86-specific constants and determine at runtime.
// These values are completely controlled by software on RISC-V platform as long
// as they supports more or equal kinds of external interrupt the [`IRQ_CHIP`]
// can supports.
pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// The [`IrqChip`] singleton.
pub static IRQ_CHIP: Once<SpinLock<IrqChip>> = Once::new();

/// Initializes the Platform-Level Interrupt Controller (PLIC).
///
/// # Safety
///
/// 1. This function should be called once and at most once at a proper timing
///    in the boot context.
/// 2. This function should be called before any other public functions of this
///    module is called.
/// 3. This function should be called after the kernel page table is activated.
pub(super) unsafe fn init(io_mem_builder: &IoMemAllocatorBuilder) {
    let device_tree = DEVICE_TREE.get().unwrap();
    let plics = Plic::from_fdt(device_tree, io_mem_builder);
    IRQ_CHIP.call_once(|| {
        SpinLock::new(IrqChip {
            plics: plics.into_boxed_slice(),
            interrupt_number_mappings: (IRQ_NUM_MIN..=IRQ_NUM_MAX)
                .map(|_| None)
                .collect::<Vec<Option<InterruptSourceHandle>>>()
                .into_boxed_slice(),
        })
    });
    IRQ_CHIP
        .get()
        .unwrap()
        .lock()
        .plics
        .iter()
        .for_each(|plic| {
            // SAFETY: The caller ensures that the kernel page table is activated.
            unsafe {
                plic.init();
            }
        });
    // SAFETY: Accessing the `sie` CSR to enable the external interrupt is
    // safe here because this function is only called during PLIC
    // initialization, and we ensure that only the external interrupt bit is
    // set without affecting other interrupt sources.
    unsafe {
        riscv::register::sie::set_sext();
    }
}

// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local() {
    // SAFETY: The safety is upheld by the caller.
    unsafe { riscv::interrupt::enable() }
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
//
// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local_and_halt() {
    // RISC-V Instruction Set Manual, Machine-Level ISA, Version 1.13 says:
    // "The WFI instruction can also be executed when interrupts are disabled. The operation of WFI
    // must be unaffected by the global interrupt bits in `mstatus` (MIE and SIE) [..]"
    //
    // So we can use `wfi` even if IRQs are disabled. Pending IRQs can still wake up the CPU, but
    // they will only occur later when we enable local IRQs.
    riscv::asm::wfi();

    // SAFETY: The safety is upheld by the caller.
    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn disable_local() {
    riscv::interrupt::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
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
/// [`map_interrupt_source_to`]: Self::map_interrupt_source_to
pub struct IrqChip {
    plics: Box<[Plic]>,
    /// Global IRQ-number-to-interrupt-source mappings.
    interrupt_number_mappings: Box<[Option<InterruptSourceHandle>]>,
}

impl IrqChip {
    /// Maps an IRQ specified by `interrupt_source` to an IRQ line.
    pub fn map_interrupt_source_to(
        &mut self,
        interrupt_source: InterruptSource,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        assert!(IS_KERNEL_PAGE_TABLE_ACTIVATED.load(Ordering::Relaxed));
        let (index, plic) = self
            .plics
            .iter_mut()
            .enumerate()
            .find(|(_, plic)| plic.phandle == interrupt_source.interrupt_parent)
            .unwrap();
        self.interrupt_number_mappings[irq_line.num() as usize] = Some(InterruptSourceHandle {
            index,
            interrupt: interrupt_source.interrupt,
        });
        plic.map_interrupt_source_to(interrupt_source.interrupt, &irq_line)?;
        // SAFETY: The kernel page table is already activated.
        unsafe {
            plic.set_priority(interrupt_source.interrupt, 1);
            plic.set_interrupt_enabled(
                CpuId::current_racy().as_usize() as u32,
                interrupt_source.interrupt,
                true,
            );
        }

        Ok(MappedIrqLine {
            irq_line,
            interrupt_source,
        })
    }

    /// Claims an external interrupt that is pending on a specific hart.
    ///
    /// It returns the software IRQ number if there's a pending interrupt on the
    /// hart, otherwise it will return `None`.
    pub(super) fn claim_interrupt(&self, hart: u32) -> Option<u32> {
        assert!(IS_KERNEL_PAGE_TABLE_ACTIVATED.load(Ordering::Relaxed));
        self.plics.iter().find_map(|plic| {
            // SAFETY: The kernel page table is already activated.
            plic.interrupt_number_mappings[unsafe { plic.claim_interrupt(hart) } as usize]
        })
    }

    /// Acknowledges the completion of an interrupt.
    pub(super) fn complete_interrupt(&self, hart: u32, irq_num: u32) {
        assert!(IS_KERNEL_PAGE_TABLE_ACTIVATED.load(Ordering::Relaxed));
        let InterruptSourceHandle { index, interrupt } = self.interrupt_number_mappings
            [irq_num as usize]
            .as_ref()
            .unwrap();
        // SAFETY: The kernel page table is already activated.
        unsafe {
            self.plics[*index].complete_interrupt(hart, *interrupt);
        }
    }

    /// Unmaps an IRQ line from the IRQ chip.
    fn unmap_irq_line_from(&mut self, irq_line: &IrqLine) {
        assert!(IS_KERNEL_PAGE_TABLE_ACTIVATED.load(Ordering::Relaxed));
        let InterruptSourceHandle { index, interrupt } = self.interrupt_number_mappings
            [irq_line.num() as usize]
            .as_ref()
            .unwrap();
        let plic = &mut self.plics[*index];
        // SAFETY: The kernel page table is already activated.
        unsafe {
            plic.set_interrupt_enabled(CpuId::current_racy().as_usize() as u32, *interrupt, false);
            plic.set_priority(*interrupt, 0);
        }
        plic.unmap_interrupt_source(*interrupt);
        self.interrupt_number_mappings[irq_line.num() as usize] = None;
    }
}

/// An [`IrqLine`] mapped to an IRQ pin managed by the [`IRQ_CHIP`].
///
/// When the object is dropped, the IRQ line will be unmapped by the IRQ chip.
pub struct MappedIrqLine {
    irq_line: IrqLine,
    interrupt_source: InterruptSource,
}

impl fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("interrupt_source", &self.interrupt_source)
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
        IRQ_CHIP
            .get()
            .unwrap()
            .lock()
            .unmap_irq_line_from(&self.irq_line)
    }
}

/// Interrupt source identifier.
///
/// An interrupt source can serve as a globally unique identifier of an IRQ.
#[derive(Clone, Copy, Debug)]
pub struct InterruptSource {
    /// Interrupt source number on one interrupt controller.
    pub interrupt: u32,
    /// Phandle of the interrupt controller it connects to.
    pub interrupt_parent: u32,
}

struct InterruptSourceHandle {
    index: usize,
    interrupt: u32,
}

pub(crate) struct IrqRemapping {
    _private: (),
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    /// Initializes the remapping entry for the specific IRQ number.
    ///
    /// This will do nothing if the entry is already initialized or interrupt
    /// remapping is disabled or not supported by the architecture.
    pub(crate) fn init(&self, _irq_num: u8) {}

    /// Gets the remapping index of the IRQ line.
    ///
    /// This method will return `None` if interrupt remapping is disabled or
    /// not supported by the architecture.
    pub(crate) fn remapping_index(&self) -> Option<u16> {
        None
    }
}

// ####### Inter-Processor Interrupts (IPIs) #######

/// Hardware-specific, architecture-dependent CPU ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        // TODO: Support SMP in RISC-V.
        Self(0)
    }
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
///
/// # Safety
///
/// The caller must ensure that the interrupt number is valid and that
/// the corresponding handler is configured correctly on the remote CPU.
/// Furthermore, invoking the interrupt handler must also be safe.
pub(crate) unsafe fn send_ipi(_hw_cpu_id: HwCpuId, _irq_num: u8, _guard: &dyn PinCurrentCpu) {
    unimplemented!()
}
