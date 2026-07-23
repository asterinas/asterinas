// SPDX-License-Identifier: MPL-2.0

//! Advanced Platform-Level Interrupt Controller (APLIC).

use alloc::{boxed::Box, vec::Vec};

use fdt::Fdt;

use super::InterruptTrigger;
use crate::{
    Error, Result,
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
};

/// A supervisor-level APLIC domain operating in MSI delivery mode.
pub(super) struct Aplic {
    phandle: u32,
    io_mem: IoMem<Sensitive>,
    num_sources: u32,
    interrupt_number_mappings: Box<[Option<u8>]>,
}

impl Aplic {
    // Register offsets and bit definitions follow the RISC-V AIA
    // specification's APLIC register layout.
    const DOMAINCFG: usize = 0x0000;
    const SOURCECFG_BASE: usize = 0x0004;
    const SETIENUM: usize = 0x1edc;
    const CLRIENUM: usize = 0x1fdc;
    const TARGET_BASE: usize = 0x3004;

    const DOMAINCFG_IE: u32 = 1 << 8;
    const DOMAINCFG_DM: u32 = 1 << 2;

    /// Discovers supervisor APLIC domains connected to the selected IMSIC.
    pub(super) fn from_fdt(
        fdt: &Fdt<'_>,
        io_mem_builder: &IoMemAllocatorBuilder,
        imsic_phandle: u32,
    ) -> Vec<Self> {
        fdt.all_nodes()
            .filter(|node| {
                node.compatible().is_some_and(|compatibles| {
                    compatibles
                        .all()
                        .any(|compatible| compatible == "riscv,aplic")
                }) && node
                    .property("msi-parent")
                    .and_then(|property| property.as_usize())
                    .is_some_and(|phandle| phandle as u32 == imsic_phandle)
            })
            .map(|node| {
                let phandle = node
                    .property("phandle")
                    .and_then(|property| property.as_usize())
                    .expect("Missing APLIC phandle") as u32;
                let num_sources = node
                    .property("riscv,num-sources")
                    .and_then(|property| property.as_usize())
                    .expect("Missing APLIC source count") as u32;
                let region = node
                    .reg()
                    .expect("Missing APLIC reg property")
                    .next()
                    .expect("Empty APLIC reg property");
                let start = region.starting_address as usize;
                let size = region.size.expect("Incomplete APLIC reg property");
                let end = start.checked_add(size).expect("APLIC MMIO range overflows");

                Self {
                    phandle,
                    io_mem: io_mem_builder.reserve(start..end, crate::mm::CachePolicy::Uncacheable),
                    num_sources,
                    interrupt_number_mappings: (0..=num_sources)
                        .map(|_| None)
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                }
            })
            .collect()
    }

    pub(super) fn phandle(&self) -> u32 {
        self.phandle
    }

    pub(super) fn init(&mut self) {
        self.write(Self::DOMAINCFG, 0);
        for source in 1..=self.num_sources {
            self.write(Self::CLRIENUM, source);
            self.write(Self::SOURCECFG_BASE + source as usize * 4 - 4, 0);
        }
        self.write(Self::DOMAINCFG, Self::DOMAINCFG_IE | Self::DOMAINCFG_DM);
    }

    pub(super) fn map_interrupt_source_to(
        &mut self,
        interrupt_source: u32,
        trigger: InterruptTrigger,
        irq_line: &IrqLine,
        msi_interrupt_id: u16,
    ) -> Result<()> {
        if interrupt_source == 0 || interrupt_source > self.num_sources {
            return Err(Error::InvalidArgs);
        }
        if self.interrupt_number_mappings[interrupt_source as usize].is_some() {
            return Err(Error::AccessDenied);
        }

        let source_mode = match trigger {
            InterruptTrigger::EdgeRising => 4,
            InterruptTrigger::EdgeFalling => 5,
            InterruptTrigger::LevelHigh => 6,
            InterruptTrigger::LevelLow => 7,
        };
        let source_config = Self::SOURCECFG_BASE + interrupt_source as usize * 4 - 4;
        let target = Self::TARGET_BASE + interrupt_source as usize * 4 - 4;

        self.write(Self::CLRIENUM, interrupt_source);
        self.write(source_config, source_mode);
        // This initial single-target setup leaves hart and guest indexes at zero
        // and writes only the IMSIC interrupt identity as the target EIID.
        self.write(target, u32::from(msi_interrupt_id));
        self.write(Self::SETIENUM, interrupt_source);
        self.interrupt_number_mappings[interrupt_source as usize] = Some(irq_line.num());
        Ok(())
    }

    pub(super) fn unmap_interrupt_source(&mut self, interrupt_source: u32) {
        if interrupt_source == 0 || interrupt_source > self.num_sources {
            return;
        }
        self.write(Self::CLRIENUM, interrupt_source);
        self.write(Self::SOURCECFG_BASE + interrupt_source as usize * 4 - 4, 0);
        self.interrupt_number_mappings[interrupt_source as usize] = None;
    }

    fn write(&mut self, offset: usize, value: u32) {
        // SAFETY: `offset` is a naturally aligned APLIC register within the
        // device-tree-provided MMIO region exclusively reserved by this driver.
        unsafe { self.io_mem.write_once(offset, &value) };
    }
}
