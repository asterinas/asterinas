// SPDX-License-Identifier: MPL-2.0

//! Interrupt chip support via ARM GICv3.

use core::ops::Deref;

use arm_gic::{
    IntId, InterruptGroup,
    gicv3::{self},
};
use spin::Once;

use super::{HwIrqLine, InterruptSource};
use crate::{Result, io::IoMemAllocatorBuilder, irq::IrqLine, mm::paddr_to_vaddr, sync::SpinLock};

/// Max number of GIC INTIDs we support for mapping.
const MAX_INTIDS: usize = 1024;

/// Maps GIC INTID → allocated software IRQ number.
/// Stored inside `IrqChip` (like RISC-V PLIC's `interrupt_number_mappings`),
/// populated by `map_fdt_pin_to`, read by `claim_interrupt()`.
/// Only forward mapping is needed — reverse mapping is eliminated because
/// `HwIrqLine` carries the INTID through `InterruptSource`.
pub struct IrqChip {
    intid_to_irq_num: SpinLock<[Option<u8>; MAX_INTIDS]>,
}

/// Interrupt source identifier on the chip.
///
/// On AArch64 there is only one GIC, so no index field is needed
/// (unlike RISC-V which may have multiple PLICs).
#[derive(Clone, Copy, Debug)]
pub struct InterruptSourceOnChip {
    pub intid: u32,
}

/// Interrupt source identifier in FDT.
///
/// For ARM GIC, `interrupt` should be the GIC INTID (already converted
/// from the FDT <type num flags> format by the caller).
#[derive(Clone, Copy, Debug)]
pub struct InterruptSourceInFdt {
    /// Phandle of the interrupt controller.
    pub interrupt_parent: u32,
    /// GIC INTID (for SPIs: FDT number + 32; for PPIs: FDT number + 16).
    pub interrupt: u32,
}

impl IrqChip {
    /// Maps an IRQ pin specified by `interrupt_source_in_fdt` to an IRQ line.
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let intid = interrupt_source_in_fdt.interrupt;
        let irq_num = irq_line.num();

        // Store INTID → irq_num forward mapping inside IrqChip
        {
            let mut map = self.intid_to_irq_num.lock();
            if map[intid as usize].is_some() {
                return Err(crate::Error::AccessDenied);
            }
            map[intid as usize] = Some(irq_num);
        }

        // Configure the GIC distributor for this SPI.
        let (gicd_base, _) = GIC_BASES.get().expect("GIC bases not initialized");
        let gicd_va = paddr_to_vaddr(*gicd_base);

        unsafe {
            // GICD_IPRIORITYR: offset 0x400, 1 byte per INTID.
            let prio_off = 0x400 + (intid as usize);
            core::ptr::write_volatile((gicd_va + prio_off) as *mut u8, 0xa8);

            // GICD_ISENABLERn: offset 0x100, 1 bit per INTID.
            let group_idx = intid / 32;
            let bit = 1u32 << (intid % 32);
            core::ptr::write_volatile(
                (gicd_va + 0x100 + (group_idx as usize) * 4) as *mut u32,
                bit,
            );
        }

        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip: InterruptSourceOnChip { intid },
        })
    }

    /// Unmaps an IRQ line from the chip, clearing the forward mapping.
    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        let mut map = self.intid_to_irq_num.lock();
        map[mapped_irq_line.interrupt_source_on_chip.intid as usize] = None;
    }

    /// Claims a pending interrupt, returning a `HwIrqLine` with both
    /// the software IRQ number and the hardware identity (INTID).
    ///
    /// For known PPIs/SGIs (timer, IPI), constructs `HwIrqLine` with
    /// `InterruptSource::Timer` or `InterruptSource::Ipi`. For mapped SPIs,
    /// uses `InterruptSource::External` with the INTID stored in
    /// `InterruptSourceOnChip`. Unmapped INTIDs are completed immediately
    /// and skipped.
    pub(crate) fn claim_interrupt() -> Option<HwIrqLine> {
        match gicv3::GicCpuInterface::get_and_acknowledge_interrupt(InterruptGroup::Group1) {
            Some(intid) => {
                let intid_val = u32::from(intid);

                // Timer PPI (INTID 30 on QEMU virt)
                if intid_val == crate::arch::timer::TIMER_PPI_INTID {
                    let irq_num = crate::arch::timer::TIMER_IRQ
                        .get()
                        .expect("Timer IRQ not initialized")
                        .num();
                    return Some(HwIrqLine::new(
                        irq_num,
                        InterruptSource::Timer { intid: intid_val },
                    ));
                }

                // SGI (INTID 0-15) — IPI
                if intid_val < 16 {
                    let irq_num = super::ipi::IPI_IRQ
                        .get()
                        .expect("IPI IRQ not initialized")
                        .num();
                    return Some(HwIrqLine::new(
                        irq_num,
                        InterruptSource::Ipi { intid: intid_val },
                    ));
                }

                // SPI or PPI: look up in IrqChip's forward mapping
                let map = IRQ_CHIP
                    .get()
                    .expect("IRQ chip not initialized")
                    .intid_to_irq_num
                    .lock();
                if let Some(irq_num) = map[intid_val as usize] {
                    Some(HwIrqLine::new(
                        irq_num,
                        InterruptSource::External(InterruptSourceOnChip { intid: intid_val }),
                    ))
                } else {
                    // Unmapped INTID: complete immediately and skip
                    complete_with_intid(intid_val);
                    None
                }
            }
            None => None,
        }
    }
}

/// Completes handling of an interrupt by writing ICC_EOIR1_EL1 with the INTID.
///
/// This is the GICv3 end-of-interrupt operation. It takes the raw INTID directly.
pub(crate) fn complete_with_intid(intid: u32) {
    if let Ok(id) = IntId::try_from(intid) {
        gicv3::GicCpuInterface::end_interrupt(id, InterruptGroup::Group1);
    }
}

/// A mapped IRQ line.
///
/// Wraps an [`IrqLine`] and carries the `InterruptSourceOnChip` (containing
/// the GIC INTID) so that `ack()` can perform EOI.
/// When dropped, the GIC distributor SPI is disabled and the forward mapping
/// is cleared.
pub struct MappedIrqLine {
    irq_line: IrqLine,
    interrupt_source_on_chip: InterruptSourceOnChip,
}

impl Deref for MappedIrqLine {
    type Target = IrqLine;
    fn deref(&self) -> &Self::Target {
        &self.irq_line
    }
}

impl core::fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("interrupt_source_on_chip", &self.interrupt_source_on_chip)
            .finish_non_exhaustive()
    }
}

impl Drop for MappedIrqLine {
    fn drop(&mut self) {
        let intid = self.interrupt_source_on_chip.intid;
        // Disable the SPI in GIC distributor
        let (gicd_base, _) = GIC_BASES.get().expect("GIC bases not initialized");
        let gicd_va = paddr_to_vaddr(*gicd_base);
        unsafe {
            // GICD_ICENABLERn: offset 0x180
            let group_idx = intid / 32;
            let bit = 1u32 << (intid % 32);
            core::ptr::write_volatile(
                (gicd_va + 0x180 + (group_idx as usize) * 4) as *mut u32,
                bit,
            );
        }
        // Clear the forward mapping inside IrqChip
        IRQ_CHIP.get().unwrap().unmap_irq_line(self);
    }
}

/// GICv3 MMIO base addresses, set during init.
pub(crate) static GIC_BASES: Once<(usize, usize)> = Once::new();

/// The IRQ chip singleton.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

/// GICv3 MMIO base addresses from FDT.
///
/// Parse GIC addresses from FDT GIC node "reg" property.
/// GICv3 typically has two reg entries: GICD (Distributor) and GICR (Redistributor).
///
/// # Panics
///
/// Panics if FDT doesn't contain a GIC node or valid reg property.
fn get_gic_base_addresses() -> (usize, usize) {
    let fdt = crate::arch::boot::DEVICE_TREE
        .get()
        .expect("FDT not initialized before GIC init");

    // Search for GIC node by scanning all nodes for compatible string.
    let intc_node = fdt
        .all_nodes()
        .find(|node| {
            if let Some(prop) = node.property("compatible") {
                let compat = prop.as_str().unwrap_or("");
                compat.contains("arm,gic-v3")
            } else {
                false
            }
        })
        .expect("FDT missing GIC interrupt controller node");

    // Parse reg property using raw_reg() API
    // GICv3 reg format: (GICD_base, GICD_size, GICR_base, GICR_size)
    // Each address/size is 64-bit (2 cells) for ARM64
    let mut reg_iter = intc_node
        .raw_reg()
        .expect("FDT GIC node missing reg property");

    // First entry: GICD (Distributor)
    let gicd_reg = reg_iter.next().expect("FDT GIC reg missing GICD entry");
    let gicd_base = parse_u64_from_be_bytes(gicd_reg.address).expect("Invalid GICD address in FDT");

    // Second entry: GICR (Redistributor)
    let gicr_reg = reg_iter.next().expect("FDT GIC reg missing GICR entry");
    let gicr_base = parse_u64_from_be_bytes(gicr_reg.address).expect("Invalid GICR address in FDT");

    crate::info!(
        "GIC: parsed from FDT - GICD={:#x}, GICR={:#x}",
        gicd_base,
        gicr_base
    );
    (gicd_base as usize, gicr_base as usize)
}

fn parse_u32_be(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Helper to parse GIC interrupt from raw FDT bytes.
///
/// The GIC uses a 3-cell format: `<type number flags>`.
/// - type 0 = SPI → INTID = number + 32
/// - type 1 = PPI → INTID = number + 16
pub fn parse_gic_intid_from_fdt(interrupts_value: &[u8]) -> Option<u32> {
    let irq_type = parse_u32_be(interrupts_value)?;
    let irq_num = parse_u32_be(&interrupts_value[4..])?;
    match irq_type {
        0 => Some(irq_num + 32), // SPI
        1 => Some(irq_num + 16), // PPI
        _ => None,
    }
}

fn parse_u64_from_be_bytes(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 8 {
        return None;
    }
    Some(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

/// Initializes the GICv3 on the BSP.
///
/// # Safety
///
/// The caller ensures this is called only once on BSP at the right time.
pub(crate) unsafe fn init_on_bsp(_io_mem_builder: &IoMemAllocatorBuilder) {
    let (gicd_base, gicr_base) = get_gic_base_addresses();
    GIC_BASES.call_once(|| (gicd_base, gicr_base));

    // Enable system register interface for ICC_* registers.
    gicv3::GicCpuInterface::enable_system_register_el1();

    // ICC_BPR1_EL1 = 0: clear binary point register to restore reset value.
    // Some firmware (UEFI/edk2) sets BPR to a value large enough to prevent
    // any pre-emptive interrupts, following Linux (gic_cpu_sys_reg_init).
    // SAFETY: Writing ICC_BPR1_EL1 is always safe.
    unsafe { core::arch::asm!("msr ICC_BPR1_EL1, xzr") };

    // ICC_CTLR_EL1 = 0: set EOImode=0 (combined priority-drop + deactivate).
    // SAFETY: Writing ICC_CTLR_EL1 is always safe.
    unsafe { core::arch::asm!("msr ICC_CTLR_EL1, xzr") };

    // ICC_AP1R0_EL1 = 0: clear active priority register for Group 1.
    // Stale firmware values could prevent new interrupts from being signaled.
    // SAFETY: Writing ICC_AP1R0_EL1 is always safe.
    unsafe { core::arch::asm!("msr ICC_AP1R0_EL1, xzr") };

    // Set priority mask to allow all priorities.
    // Linux uses 0xf0 instead of 0xff, but on NS-only systems the bottom
    // 4 bits are RAZ/WI, so 0xff is equivalent.
    gicv3::GicCpuInterface::set_priority_mask(0xf0);

    // Wake up redistributor, init distributor, and configure timer PPI.
    // Use linear-mapped VAs since kernel page table has linear mapping.
    unsafe {
        let gicr_va = paddr_to_vaddr(gicr_base);
        let gicd_va = paddr_to_vaddr(gicd_base);

        // GICR_WAKER (offset 0x0014): clear ProcessorSleep (bit 1), then
        // poll until ChildrenAsleep (bit 2) clears (Linux gic_enable_redist).
        let waker_addr = (gicr_va + 0x0014) as *mut u32;
        core::ptr::write_volatile(waker_addr, core::ptr::read_volatile(waker_addr) & !2u32);
        // Poll ChildrenAsleep until it clears (wakeup complete).
        while core::ptr::read_volatile(waker_addr as *const u32) & 4 != 0 {}

        // GICD_CTLR (offset 0x0000): disable distributor first, then enable.
        // Follow Linux: write 0 first, then write desired value.
        core::ptr::write_volatile(gicd_va as *mut u32, 0);
        // Poll RWP (bit 31) until the write completes.
        while core::ptr::read_volatile(gicd_va as *const u32) & 0x8000_0000 != 0 {}
        // Read current value (preserves DS and ARE bits set by QEMU reset)
        // and set EnableGrp1NS (bit 1). In GICv3 with ARE enabled, this
        // enables Group 1 Non-secure interrupts.
        let ctlr = core::ptr::read_volatile(gicd_va as *const u32);
        core::ptr::write_volatile(gicd_va as *mut u32, ctlr | 0x2);

        // GICR_SGI base = GICR base + 0x10000 (SZ_64K).
        let gicr_sgi_va = gicr_va + 0x10000;

        // GICR_ICENABLER0 (SGI base + 0x180): disable all SGIs/PPIs first.
        core::ptr::write_volatile((gicr_sgi_va + 0x180) as *mut u32, !0u32);

        // GICR_ICACTIVER0 (SGI base + 0x380): deactivate all SGIs/PPIs.
        core::ptr::write_volatile((gicr_sgi_va + 0x380) as *mut u32, !0u32);

        // GICR_IGROUPR0 (SGI base + 0x80): assign all SGIs/PPIs to Group-1.
        // Linux (irq-gic-v3.c: gic_cpu_init writes ~0 to GICR_IGROUPR0).
        core::ptr::write_volatile((gicr_sgi_va + 0x80) as *mut u32, !0u32);

        // GICR_IPRIORITYR0 (SGI base + 0x400): set priority for all SGIs/PPIs.
        // Linux gic_cpu_config writes dist_prio_irq (0xa8) to all SGI/PPI priorities.
        for i in (0..32usize).step_by(4) {
            core::ptr::write_volatile((gicr_sgi_va + 0x400 + i) as *mut u32, 0xa8a8a8a8u32);
        }

        // GICR_ISENABLER0 (SGI base + 0x100): enable PPI 30 (CNTPIRQ).
        // ISENABLER0 uses full INTID numbering (0-31), NOT PPI-relative.
        // PPI 30 = INTID 30 = bit 30.
        core::ptr::write_volatile((gicr_sgi_va + 0x100) as *mut u32, 1u32 << 30);

        // Configure SPIs (INTID 32..MAX), following Linux gic_dist_init:
        //   - GICD_IGROUPRn: ~0  (all SPIs to Group-1)
        //   - GICD_ICENABLERn: ~0 (disable all SPIs)
        //   - GICD_ICACTIVERn: ~0 (deactivate all SPIs)
        let gicd_typer = core::ptr::read_volatile((gicd_va + 0x0008) as *const u32);
        let it_lines = (gicd_typer & 0x1f) as usize; // ITLinesNumber = max SPI INTID / 32 - 1
        let num_spi_groups = it_lines; // Each group covers 32 SPIs (INTIDs 32..32*it_lines+31)
        for n in 1..=num_spi_groups {
            let off = n * 4;
            core::ptr::write_volatile((gicd_va + 0x0080 + off) as *mut u32, !0u32); // IGROUPRn
            core::ptr::write_volatile((gicd_va + 0x0180 + off) as *mut u32, !0u32); // ICENABLERn
            core::ptr::write_volatile((gicd_va + 0x0380 + off) as *mut u32, !0u32); // ICACTIVERn
        }
        // Set SPI priorities (Linux gic_dist_config: dist_prio_irq = 0xa8).
        // GICD_IPRIORITYR: skip first 32 bytes (SGIs/PPIs), write 0xa8 per byte.
        let num_spis = num_spi_groups * 32;
        for i in (0..num_spis).step_by(4) {
            core::ptr::write_volatile((gicd_va + 0x0400 + 32 + i) as *mut u32, 0xa8a8a8a8u32);
        }
    }

    // Enable Group 1 at CPU interface LAST, after distributor and redistributor
    // are fully configured (matching Linux gic_cpu_sys_reg_init order).
    gicv3::GicCpuInterface::enable_group1(true);

    IRQ_CHIP.call_once(|| IrqChip {
        intid_to_irq_num: SpinLock::new([None; MAX_INTIDS]),
    });
}

/// Initializes the GICv3 on an AP.
///
/// # Safety
///
/// The caller ensures this is called only once on each AP.
pub(crate) unsafe fn init_on_ap() {
    gicv3::GicCpuInterface::enable_system_register_el1();

    // SAFETY: Writing ICC system registers is always safe.
    unsafe { core::arch::asm!("msr ICC_BPR1_EL1, xzr") };
    unsafe { core::arch::asm!("msr ICC_CTLR_EL1, xzr") };
    unsafe { core::arch::asm!("msr ICC_AP1R0_EL1, xzr") };

    gicv3::GicCpuInterface::set_priority_mask(0xf0);

    // FIXME: Wake this AP's redistributor, configure GICR_SGI registers,
    // then call enable_group1 last (matching init_on_bsp order).

    // Enable Group 1 at CPU interface last.
    gicv3::GicCpuInterface::enable_group1(true);
}
