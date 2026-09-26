// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};
use core::{
    arch::asm,
    ops::Range,
    sync::atomic::{AtomicU8, Ordering},
};

use fdt::Fdt;

use super::{InterruptSourceInFdt, InterruptSourceOnChip};
use crate::{
    Error, Result,
    arch::irq::{HwIrqLine, IRQ_NUM_INVALID},
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};

/// The Generic Interrupt Controller (GIC) for ARM.
pub(super) struct Gic {
    phandle: u32,
    inner: SpinLock<Inner, LocalIrqDisabled>,
    /// Per-GIC interrupt-source-to-IRQ-number mappings.
    interrupt_number_mappings: Box<[AtomicU8]>,
}

/// GIC implementation.
///
/// A GIC consists of two parts: a distributor and a set of redistributors for each processing
/// element (PE).
///  - The distributor routes a shared peripheral interrupt (SPI) to one of the PEs that are
///    configured to handle it.
///  - Redistributors route private peripheral interrupts (PPIs) that belong to a specific PE.
struct Inner {
    distributor: Distributor,
    redistributor: Redistributor,
}

impl Gic {
    /// A special value in `interrupt_number_mappings` indicating that the interrupt is not mapped.
    const UNMAPPED_INTERRUPT: u8 = {
        assert!(0xFF > crate::arch::irq::IRQ_NUM_MAX);
        0xFF
    };

    pub(super) fn from_fdt(
        fdt: &Fdt,
        io_mem_allocator_builder: &mut IoMemAllocatorBuilder,
    ) -> Option<Self> {
        let node = fdt.find_compatible(&["arm,gic-v3"])?;

        let phandle = node
            .property("phandle")
            .and_then(|phandle| phandle.as_usize())
            .expect("Failed to read 'phandle' property from GIC node") as u32;

        let mut regs = node
            .reg()
            .expect("Failed to read 'reg' property from GIC node");
        let mut next_reg = || {
            let reg = regs.next().expect("Empty 'reg' property found in GIC node");

            let addr = reg.starting_address as usize;
            let size = reg
                .size
                .expect("Incomplete 'reg' property found in GIC node");

            io_mem_allocator_builder.reserve(addr..addr + size, crate::mm::CachePolicy::Uncacheable)
        };

        let mut distributor = {
            let io_mem = next_reg();
            Distributor(DistributorBase {
                offset: Distributor::BASE_OFFSET,
                io_mem,
            })
        };
        let mut redistributor = {
            let io_mem = next_reg();
            Redistributor(DistributorBase {
                offset: Redistributor::BASE_OFFSET,
                io_mem,
            })
        };

        distributor.init();
        redistributor.init();

        // SAFETY: This is part of the GIC configuration, as documented in the "CPU interface
        // configuration" section at
        // <https://support.arm.com/documentation/198123/0302/Configuring-the-Arm-GIC>.
        unsafe {
            asm!(
                "mrs {tmp}, icc_sre_el1",
                "orr {tmp}, {tmp}, #1", // System Register Enable (SRE)
                "msr icc_sre_el1, {tmp}",

                "mov {tmp}, #0xff", // Lowest priority
                "msr icc_pmr_el1, {tmp}",
                "mov {tmp}, #7", // No preemption
                "msr icc_bpr1_el1, {tmp}",

                "mrs {tmp}, icc_ctlr_el1",
                "and {tmp}, {tmp}, #~2", // EOI deactivates the interrupt
                "msr icc_ctlr_el1, {tmp}",

                "mov {tmp}, #1", // Enable
                "msr icc_igrpen1_el1, {tmp}",

                tmp = out(reg) _
            );
        }

        let inner = Inner {
            distributor,
            redistributor,
        };
        let mappings = (0..inner.distributor.get_interrupt_count())
            .map(|_| AtomicU8::new(Self::UNMAPPED_INTERRUPT))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Some(Self {
            phandle,
            inner: SpinLock::new(inner),
            interrupt_number_mappings: mappings,
        })
    }

    pub(super) fn map_interrupt_source_to(
        &self,
        interrupt_source: InterruptSourceInFdt,
        irq_line: &IrqLine,
    ) -> Result<InterruptSourceOnChip> {
        const TYPE_SPI: u32 = 0;
        const TYPE_PPI: u32 = 1;

        const TRIGGER_MASK: u32 = 0xF;
        const TRIGGER_EDGE: u32 = 1;
        const TRIGGER_LEVEL: u32 = 4;

        if interrupt_source.interrupt_parent != self.phandle {
            return Err(Error::InvalidArgs);
        }

        let typ = interrupt_source.arguments[0];
        let id = interrupt_source.arguments[1];
        let flags = interrupt_source.arguments[2];

        let is_spi = if typ == TYPE_SPI {
            true
        } else if typ == TYPE_PPI {
            false
        } else {
            return Err(Error::InvalidArgs);
        };

        let is_edge = if flags & TRIGGER_MASK == TRIGGER_EDGE {
            true
        } else if flags & TRIGGER_MASK == TRIGGER_LEVEL {
            false
        } else {
            return Err(Error::InvalidArgs);
        };

        let mut inner = self.inner.lock();

        let (base_id, max_id) = if is_spi {
            (Distributor::BASE_SPI, Distributor::MAX_SPI)
        } else {
            (Redistributor::BASE_PPI, Redistributor::MAX_PPI)
        };
        let intid = base_id.checked_add(id).ok_or(Error::InvalidArgs)?;
        if intid > max_id || intid as usize >= self.interrupt_number_mappings.len() {
            return Err(Error::InvalidArgs);
        }

        if self.interrupt_number_mappings[intid as usize].load(Ordering::Relaxed) != IRQ_NUM_INVALID
        {
            return Err(Error::AccessDenied);
        }
        self.interrupt_number_mappings[intid as usize].store(irq_line.num(), Ordering::Relaxed);

        if is_spi {
            inner.distributor.set_affinity(intid);
            inner.distributor.set_priority(intid, 0x80);
            inner.distributor.set_group1(intid);
            inner.distributor.set_edge_or_level(intid, is_edge);
            inner.distributor.set_enabled(intid, true);
        } else {
            inner.redistributor.set_priority(intid, 0x80);
            inner.redistributor.set_group1(intid);
            inner.redistributor.set_edge_or_level(intid, is_edge);
            inner.redistributor.set_enabled(intid, true);
        }

        Ok(InterruptSourceOnChip {
            interrupt_parent: self.phandle,
            interrupt: intid,
        })
    }

    pub(super) fn unmap_interrupt_source(&self, interrupt_source: InterruptSourceOnChip) {
        assert_eq!(interrupt_source.interrupt_parent, self.phandle);

        let mut inner = self.inner.lock();

        let intid = interrupt_source.interrupt;
        if intid >= Distributor::BASE_SPI {
            inner.distributor.set_enabled(intid, false);
        } else {
            inner.redistributor.set_enabled(intid, false);
        };

        self.interrupt_number_mappings[intid as usize]
            .store(Self::UNMAPPED_INTERRUPT, Ordering::Relaxed);
    }

    pub(super) fn claim_interrupt(&self) -> Option<HwIrqLine> {
        const RESERVED_INTIDS: Range<usize> = 1020..1024;

        let (irq_num, iar1) = loop {
            let iar1: usize;
            // SAFETY: It is safe to read from the Interrupt Controller Interrupt Acknowledge Register.
            unsafe { asm!("mrs {}, icc_iar1_el1", out(reg) iar1) };

            if RESERVED_INTIDS.contains(&iar1) {
                return None;
            }

            let irq_num = self.interrupt_number_mappings[iar1].load(Ordering::Relaxed);
            if irq_num == Self::UNMAPPED_INTERRUPT {
                // SAFETY: It is safe to write to the Interrupt Controller End Of Interrupt Register.
                unsafe { asm!("msr icc_eoir1_el1, {}", in(reg) iar1) }
                continue;
            }

            break (irq_num, iar1);
        };

        Some(HwIrqLine {
            irq_num,
            source: InterruptSourceOnChip {
                interrupt_parent: self.phandle,
                interrupt: iar1 as u32,
            },
        })
    }

    pub(super) fn complete_interrupt(&self, interrupt_source: InterruptSourceOnChip) {
        assert_eq!(interrupt_source.interrupt_parent, self.phandle);

        // SAFETY: It is safe to write to the Interrupt Controller End Of Interrupt Register.
        unsafe { asm!("msr icc_eoir1_el1, {}", in(reg) interrupt_source.interrupt as u64) }
    }
}

struct DistributorBase {
    offset: usize,
    io_mem: IoMem<Sensitive>,
}

impl DistributorBase {
    const GICD_IGROUPR: usize = 0x0080;
    const GICD_ISENABLER: usize = 0x0100;
    const GICD_ICENABLER: usize = 0x0180;
    const GICD_IPRIORITYR: usize = 0x0400;
    const GICD_ICFGR: usize = 0x0c00;

    unsafe fn set_priority(&mut self, intid: u32, prio: u8) {
        let offset = self.offset + Self::GICD_IPRIORITYR + (intid as usize & !3);
        let shift = (intid & 3) * 8;
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let mut val = self.io_mem.read_once::<u32>(offset);
            val &= !(0xff << shift);
            val |= (prio as u32) << shift;
            self.io_mem.write_once(offset, &val);
        }
    }

    unsafe fn set_group1(&mut self, intid: u32) {
        let offset = self.offset + Self::GICD_IGROUPR + (intid as usize / 32) * 4;
        let bit = 1u32 << (intid % 32);
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let mut val = self.io_mem.read_once::<u32>(offset);
            val |= bit;
            self.io_mem.write_once(offset, &val);
        }
    }

    unsafe fn set_edge_or_level(&mut self, intid: u32, is_edge: bool) {
        let offset = self.offset + Self::GICD_ICFGR + (intid as usize / 16) * 4;
        let bit = 1u32 << ((intid % 16) * 2 + 1);
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let mut val = self.io_mem.read_once::<u32>(offset);
            if is_edge {
                val |= bit;
            } else {
                val &= !bit;
            }
            self.io_mem.write_once(offset, &val);
        }
    }

    unsafe fn set_enabled(&mut self, intid: u32, is_enabled: bool) {
        let offset = if is_enabled {
            self.offset + Self::GICD_ISENABLER + (intid as usize / 32) * 4
        } else {
            self.offset + Self::GICD_ICENABLER + (intid as usize / 32) * 4
        };
        let bit = 1u32 << (intid % 32);
        // SAFETY: The safety is upheld by the caller.
        unsafe { self.io_mem.write_once(offset, &bit) };
    }
}

struct Distributor(DistributorBase);

impl Distributor {
    const BASE_OFFSET: usize = 0;

    const GICD_CTLR: usize = 0x0000;
    const GICD_TYPER: usize = 0x0004;
    const GICD_IROUTER: usize = 0x6000;

    const BASE_SPI: u32 = 32;
    const MAX_SPI: u32 = 1019;

    fn init(&mut self) {
        const ARE: u32 = 1 << 4; // Affinity Routing Enable.
        const ENABLE_GRP1: u32 = 1 << 1;
        const ENABLE_GRP0: u32 = 1 << 0;

        // SAFETY: This is part of the GIC configuration, as documented in the "Global Settings"
        // section at
        // <https://support.arm.com/documentation/198123/0302/Configuring-the-Arm-GIC>.
        unsafe {
            let mut ctrl = self.0.io_mem.read_once::<u32>(Self::GICD_CTLR);
            ctrl &= !(ENABLE_GRP1 | ENABLE_GRP0);
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);

            let mut ctrl = self.0.io_mem.read_once::<u32>(Self::GICD_CTLR);
            ctrl |= ARE;
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);

            let mut ctrl = self.0.io_mem.read_once::<u32>(Self::GICD_CTLR);
            ctrl |= ENABLE_GRP1;
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);
        }
    }

    fn get_interrupt_count(&self) -> usize {
        // SAFETY: It is safe to read from the Interrupt Controller Type Register.
        let typ = unsafe { self.0.io_mem.read_once::<u32>(Self::GICD_TYPER) };
        let cnt = ((typ & 31) + 1) * 32;
        cnt.min(Self::MAX_SPI + 1) as usize
    }

    fn set_affinity(&mut self, intid: u32) {
        // "Interrupts routed to any PE defined as a participating node."
        const ROUTE_ANY_PE: u32 = 1 << 31;

        assert!(intid <= Self::MAX_SPI);
        let offset = Self::GICD_IROUTER + intid as usize * 8;
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.io_mem.write_once::<u32>(offset, &ROUTE_ANY_PE) };
    }

    fn set_priority(&mut self, intid: u32, prio: u8) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_priority(intid, prio) };
    }

    fn set_group1(&mut self, intid: u32) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_group1(intid) };
    }

    fn set_edge_or_level(&mut self, intid: u32, is_edge: bool) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_edge_or_level(intid, is_edge) };
    }

    fn set_enabled(&mut self, intid: u32, is_enabled: bool) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_enabled(intid, is_enabled) };
    }
}

struct Redistributor(DistributorBase);

impl Redistributor {
    const BASE_OFFSET: usize = 0x10000;

    const GICR_WAKER: usize = 0x0014;

    const BASE_PPI: u32 = 16;
    const MAX_PPI: u32 = 31;

    fn init(&mut self) {
        const PROCESSOR_SLEEP: u32 = 1 << 1;
        const CHILDREN_ASLEEP: u32 = 1 << 2;

        // SAFETY: This is part of the GIC configuration, as documented in the "Redistributor
        // configuration" section at
        // <https://support.arm.com/documentation/198123/0302/Configuring-the-Arm-GIC>.
        unsafe {
            let mut waker = self.0.io_mem.read_once::<u32>(Self::GICR_WAKER);
            waker &= !PROCESSOR_SLEEP;
            self.0.io_mem.write_once(Self::GICR_WAKER, &waker);

            loop {
                let waker = self.0.io_mem.read_once::<u32>(Self::GICR_WAKER);
                if waker & CHILDREN_ASLEEP == 0 {
                    break;
                }
                core::hint::spin_loop();
            }
        }
    }

    fn set_priority(&mut self, intid: u32, prio: u8) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_priority(intid, prio) };
    }

    fn set_group1(&mut self, intid: u32) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_group1(intid) };
    }

    fn set_edge_or_level(&mut self, intid: u32, is_edge: bool) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_edge_or_level(intid, is_edge) };
    }

    fn set_enabled(&mut self, intid: u32, is_enabled: bool) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_enabled(intid, is_enabled) };
    }
}
