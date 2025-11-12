// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};
use core::{
    arch::asm,
    ops::Range,
    sync::atomic::{AtomicU8, Ordering},
};

use fdt::{
    Fdt,
    node::{FdtNode, NodeProperty},
};

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
    pub(super) fn from_fdt(
        fdt: &Fdt,
        io_mem_allocator_builder: &mut IoMemAllocatorBuilder,
    ) -> Option<Self> {
        let node = fdt.find_compatible(&["arm,gic-v3"])?;

        let phandle = node
            .property("phandle")
            .and_then(NodeProperty::as_usize)
            .expect("Failed to read 'phandle' property from GIC node") as u32;

        // SAFETY: `node` is a GICv3 node from the device tree.
        let (mut distributor, mut redistributor) =
            unsafe { Self::find_distributor_and_redistrbutors(node, io_mem_allocator_builder) };

        distributor.init();
        redistributor.init();

        Self::init_cpu_interfaces();

        let num_interrupts = distributor.read_interrupt_count();

        // Disable all interrupts.
        for intid in 0..=Redistributor::MAX_PPI {
            redistributor.set_enabled(intid, false);
        }
        for intid in (Redistributor::MAX_PPI + 1)..(num_interrupts as u32) {
            distributor.set_enabled(intid, false);
        }

        let inner = Inner {
            distributor,
            redistributor,
        };
        let mappings = (0..num_interrupts)
            .map(|_| AtomicU8::new(IRQ_NUM_INVALID))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Some(Self {
            phandle,
            inner: SpinLock::new(inner),
            interrupt_number_mappings: mappings,
        })
    }

    /// # Safety
    ///
    /// The caller must ensure that `node` is a GICv3 node from the device tree.
    unsafe fn find_distributor_and_redistrbutors(
        node: FdtNode,
        io_mem_allocator_builder: &mut IoMemAllocatorBuilder,
    ) -> (Distributor, Redistributor) {
        let mut regs = node
            .reg()
            .expect("Failed to read 'reg' property from GIC node");
        let mut next_reg = || {
            let reg = regs.next().expect("Empty 'reg' property found in GIC node");

            let addr = reg.starting_address as usize;
            let size = reg
                .size
                .expect("Incomplete 'reg' property found in GIC node");

            (addr, size)
        };

        let distributor = {
            let (addr, size) = next_reg();
            let io_mem = io_mem_allocator_builder
                .reserve(addr..addr + size, crate::mm::CachePolicy::Uncacheable);
            Distributor(DistributorBase { io_mem })
        };
        let redistributor = {
            // Redistributors are per-CPU. We will find the one associated with the BSP.
            let mut bsp_redistributor = None;

            let bsp_affinity = {
                let affinity = Self::read_affinity();
                (
                    // Bits [23:0]: Aff2, Aff1, Aff0.
                    (affinity & ((1 << 24) - 1))
                    // Bits [39:32]: Aff3.
                    | ((affinity >> 32) << 24)
                ) as u32
            };

            let redistributor_regions = node
                .property("#redistributor-regions")
                .and_then(NodeProperty::as_usize)
                // "Required if more than one such region is present."
                // Reference:
                // <https://elixir.bootlin.com/linux/v7.2.8/source/Documentation/devicetree/bindings/interrupt-controller/arm,gic-v3.yaml#L102-L107>
                .unwrap_or(1);
            let redistributor_stride = node
                .property("redistributor-stride")
                .and_then(NodeProperty::as_usize)
                // In GICv3, a redistributor spans 128 KiB. However, this is no longer true for
                // later versions of GIC that support virtual LPIs. See the VLPIS bit in GICR_TYPER
                // for more information.
                .unwrap_or(0x20000);

            for _ in 0..redistributor_regions {
                let (mut addr, mut size) = next_reg();

                while size >= redistributor_stride {
                    let io_mem = io_mem_allocator_builder.reserve(
                        addr..addr + redistributor_stride,
                        crate::mm::CachePolicy::Uncacheable,
                    );
                    addr += redistributor_stride;
                    size -= redistributor_stride;

                    let redistributor = Redistributor(DistributorBase { io_mem });
                    let is_last = redistributor.read_is_last();

                    if redistributor.read_affinity() == bsp_affinity {
                        if bsp_redistributor.is_some() {
                            crate::error!(
                                "Ignoring duplicate GIC redistributors found for the BSP"
                            );
                        } else {
                            bsp_redistributor = Some(redistributor);
                        }
                    }

                    if is_last {
                        break;
                    }
                }
            }

            bsp_redistributor.expect("No GIC redistributors found for the BSP")
        };

        (distributor, redistributor)
    }

    fn init_cpu_interfaces() {
        // SAFETY: This is part of the GIC configuration, as documented in the "CPU interface
        // configuration" section at
        // <https://support.arm.com/documentation/198123/0302/Configuring-the-Arm-GIC>.
        unsafe {
            asm!(
                "mrs {tmp}, icc_sre_el1",
                "orr {tmp}, {tmp}, #1", // System Register Enable (SRE)
                "msr icc_sre_el1, {tmp}",
                "isb",

                "mov {tmp}, #0xff", // Lowest priority
                "msr icc_pmr_el1, {tmp}",
                "mov {tmp}, #7", // No preemption
                "msr icc_bpr1_el1, {tmp}",
                "isb",

                "mrs {tmp}, icc_ctlr_el1",
                "and {tmp}, {tmp}, #~2", // EOI deactivates the interrupt
                "msr icc_ctlr_el1, {tmp}",
                "isb",

                "mov {tmp}, #1", // Enable
                "msr icc_igrpen1_el1, {tmp}",
                "isb",

                tmp = out(reg) _
            );
        }
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

        // The lower value is prioritized. It must be smaller than the value configured in the CPU
        // Interface Priority Mask Register (GICC_PMR).
        const DEFAULT_PRIORITY: u8 = 0x80;

        if is_spi {
            inner.distributor.set_affinity(intid, Self::read_affinity());
            inner.distributor.set_priority(intid, DEFAULT_PRIORITY);
            inner.distributor.set_group1(intid);
            inner.distributor.set_edge_or_level(intid, is_edge);
            inner.distributor.set_enabled(intid, true);
        } else {
            inner.redistributor.set_priority(intid, DEFAULT_PRIORITY);
            inner.redistributor.set_group1(intid);
            inner.redistributor.set_edge_or_level(intid, is_edge);
            inner.redistributor.set_enabled(intid, true);
        }

        Ok(InterruptSourceOnChip {
            interrupt_parent: self.phandle,
            interrupt: intid,
        })
    }

    fn read_affinity() -> u64 {
        let mpidr: u64;
        // SAFETY: It is safe to read from the Multiprocessor Affinity Register.
        unsafe {
            asm!(
                "mrs {mpidr}, mpidr_el1",
                mpidr = out(reg) mpidr,
            );
        }
        mpidr
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

        self.interrupt_number_mappings[intid as usize].store(IRQ_NUM_INVALID, Ordering::Relaxed);
    }

    pub(super) fn claim_interrupt(&self) -> Option<HwIrqLine> {
        /// The INTIDs that the GIC architecture reserves for special purposes.
        ///
        /// These INTIDs do not require an end of interrupt or deactivation.
        const RESERVED_INTIDS: Range<usize> = 1020..1024;

        let (irq_num, iar1) = loop {
            let iar1: usize;
            // SAFETY: It is safe to read from the Interrupt Controller Interrupt Acknowledge Register.
            unsafe { asm!("mrs {}, icc_iar1_el1", out(reg) iar1) };

            if RESERVED_INTIDS.contains(&iar1) {
                return None;
            }

            let irq_num = self
                .interrupt_number_mappings
                .get(iar1)
                .map(|mapping| mapping.load(Ordering::Relaxed))
                .unwrap_or(IRQ_NUM_INVALID);
            if irq_num == IRQ_NUM_INVALID {
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

/// A common part shared by the distributor and the redistributors.
///
/// This contains registers defined for both the distributor and the redistributors. Note that these
/// start at different offsets from the beginning of the MMIO region: [`Distributor::BASE_OFFSET`]
/// and [`Redistributor::BASE_OFFSET`].
struct DistributorBase {
    io_mem: IoMem<Sensitive>,
}

impl DistributorBase {
    const GICD_IGROUPR: usize = 0x0080;
    const GICD_ISENABLER: usize = 0x0100;
    const GICD_ICENABLER: usize = 0x0180;
    const GICD_IPRIORITYR: usize = 0x0400;
    const GICD_ICFGR: usize = 0x0c00;

    unsafe fn set_priority(&mut self, base_offset: usize, intid: u32, prio: u8) {
        let offset = base_offset + Self::GICD_IPRIORITYR + (intid as usize & !3);
        let shift = (intid & 3) * 8;
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let mut val = self.io_mem.read_once::<u32>(offset);
            val &= !(0xff << shift);
            val |= (prio as u32) << shift;
            self.io_mem.write_once(offset, &val);
        }
    }

    unsafe fn set_group1(&mut self, base_offset: usize, intid: u32) {
        let offset = base_offset + Self::GICD_IGROUPR + (intid as usize / 32) * 4;
        let bit = 1u32 << (intid % 32);
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let mut val = self.io_mem.read_once::<u32>(offset);
            val |= bit;
            self.io_mem.write_once(offset, &val);
        }
    }

    unsafe fn set_edge_or_level(&mut self, base_offset: usize, intid: u32, is_edge: bool) {
        let offset = base_offset + Self::GICD_ICFGR + (intid as usize / 16) * 4;
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

    unsafe fn set_enabled(&mut self, base_offset: usize, intid: u32, is_enabled: bool) {
        let offset = if is_enabled {
            base_offset + Self::GICD_ISENABLER + (intid as usize / 32) * 4
        } else {
            base_offset + Self::GICD_ICENABLER + (intid as usize / 32) * 4
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
        const RWP: u32 = 1 << 31; // Register Write Pending.
        const ENABLE_GRP1: u32 = 1 << 1;
        const ENABLE_GRP0: u32 = 1 << 0;

        let read_cltr_until_pending_cleared = || loop {
            // SAFETY: It is safe to read the Distributor Control Register.
            let ctrl = unsafe { self.0.io_mem.read_once::<u32>(Self::GICD_CTLR) };
            if ctrl & RWP == 0 {
                break ctrl;
            }
            core::hint::spin_loop();
        };

        // SAFETY: This is part of the GIC configuration, as documented in the "Global Settings"
        // section at
        // <https://support.arm.com/documentation/198123/0302/Configuring-the-Arm-GIC>.
        unsafe {
            let mut ctrl = read_cltr_until_pending_cleared();
            ctrl &= !(ENABLE_GRP1 | ENABLE_GRP0);
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);

            let mut ctrl = read_cltr_until_pending_cleared();
            ctrl |= ARE;
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);

            let mut ctrl = read_cltr_until_pending_cleared();
            ctrl |= ENABLE_GRP1;
            self.0.io_mem.write_once::<u32>(Self::GICD_CTLR, &ctrl);
        }
    }

    fn read_interrupt_count(&self) -> usize {
        // SAFETY: It is safe to read from the Interrupt Controller Type Register.
        let typ = unsafe { self.0.io_mem.read_once::<u32>(Self::GICD_TYPER) };
        // `cnt` is the decoded value of `ITLinesNumber`, which is the maximum SPI supported.
        let cnt = ((typ & 31) + 1) * 32;
        cnt.min(Self::MAX_SPI + 1) as usize
    }

    fn set_affinity(&mut self, intid: u32, affinity: u64) {
        // Bits [23:0]: Aff2, Aff1, Aff0.
        // Bits [39:32]: Aff3.
        let affinity = affinity & 0xff00ffffff;

        assert!(intid <= Self::MAX_SPI);
        let offset = Self::GICD_IROUTER + intid as usize * 8;
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe {
            self.0.io_mem.write_once::<u32>(offset, &(affinity as u32));
            self.0
                .io_mem
                .write_once::<u32>(offset + size_of::<u32>(), &((affinity >> 32) as u32));
        }
    }

    fn set_priority(&mut self, intid: u32, prio: u8) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_priority(Self::BASE_OFFSET, intid, prio) };
    }

    fn set_group1(&mut self, intid: u32) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_group1(Self::BASE_OFFSET, intid) };
    }

    fn set_edge_or_level(&mut self, intid: u32, is_edge: bool) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_edge_or_level(Self::BASE_OFFSET, intid, is_edge) };
    }

    fn set_enabled(&mut self, intid: u32, is_enabled: bool) {
        assert!(intid <= Self::MAX_SPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_enabled(Self::BASE_OFFSET, intid, is_enabled) };
    }
}

struct Redistributor(DistributorBase);

impl Redistributor {
    const BASE_OFFSET: usize = 0x10000;

    const GICR_TYPER: usize = 0x0008;
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

    fn read_is_last(&self) -> bool {
        // SAFETY: It is safe to read from the Redistributor Type Register.
        let typ = unsafe { self.0.io_mem.read_once::<u32>(Self::GICR_TYPER) };
        // Bit [4]: Last. Indicates whether this redistributor is the highest-numbered redistributor
        // in a series of contiguous redistributor pages.
        typ & (1 << 4) != 0
    }

    fn read_affinity(&self) -> u32 {
        // SAFETY: It is safe to read from the Redistributor Type Register.
        unsafe {
            // Bits [63:32]: Affinity_Value.
            self.0.io_mem.read_once(Self::GICR_TYPER + size_of::<u32>())
        }
    }

    fn set_priority(&mut self, intid: u32, prio: u8) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_priority(Self::BASE_OFFSET, intid, prio) };
    }

    fn set_group1(&mut self, intid: u32) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_group1(Self::BASE_OFFSET, intid) };
    }

    fn set_edge_or_level(&mut self, intid: u32, is_edge: bool) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_edge_or_level(Self::BASE_OFFSET, intid, is_edge) };
    }

    fn set_enabled(&mut self, intid: u32, is_enabled: bool) {
        assert!(intid <= Self::MAX_PPI);
        // SAFETY: We've checked that the interrupt ID is valid. It is safe to set this property of
        // a valid interrupt.
        unsafe { self.0.set_enabled(Self::BASE_OFFSET, intid, is_enabled) };
    }
}
