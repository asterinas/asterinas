// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU16;

use bit_field::BitField;
use fdt::{node::FdtNode, Fdt};
use riscv::register::scause::Interrupt;
use spin::Once;

use crate::{arch::boot::DEVICE_TREE, cpu::CpuId, cpu_local, io_mem::IoMem, mm::VmIoOnce, trap};

const PRIORITY_BASE: usize = 0x000000;
const PRIORITY_PER_SOURCE: usize = 0x000004;
const ENABLE_BASE: usize = 0x002000;
const ENABLE_PER_HART: usize = 0x000080;
const CONTEXT_BASE: usize = 0x200000;
const CONTEXT_PER_HART: usize = 0x001000;
const THRESHOLD_OFFSET: usize = 0x000000;
const CLAIM_OFFSET: usize = 0x000004;

cpu_local! {
    static PLIC_HANDLER: Once<PlicHandler> = Once::new();
}

static PLIC_IOMEM: Once<IoMem> = Once::new();

pub struct PlicHandler {
    pub context_id: usize,
    pub hart_base: usize,
    pub enable_base: usize,
}

pub fn set_priority(id: u16, priority: u32) {
    let io_mem = PLIC_IOMEM.get().unwrap();
    let offset = PRIORITY_BASE + id as usize * PRIORITY_PER_SOURCE;
    io_mem.write_once(offset, &priority).unwrap();
}

pub fn set_interrupt_enabled_on(cpu: CpuId, id: u16, enabled: bool) {
    let handler = PLIC_HANDLER.get_on_cpu(cpu).get().unwrap();
    let io_mem = PLIC_IOMEM.get().unwrap();

    let offset = handler.enable_base + (id as usize / 32) * 4;
    let mut value: u32 = io_mem.read_once(offset).unwrap();
    value.set_bit(id as usize % 32, enabled);
    io_mem.write_once(offset, &value).unwrap();
}

pub fn claim_interrupt() -> Option<NonZeroU16> {
    let io_mem = PLIC_IOMEM.get().unwrap();
    let irq_guard = trap::disable_local();

    let offset = PLIC_HANDLER.get_with(&irq_guard).get().unwrap().hart_base + CLAIM_OFFSET;
    NonZeroU16::new(io_mem.read_once::<u32>(offset).unwrap() as u16)
}

pub fn complete_interrupt(id: u16) {
    let io_mem = PLIC_IOMEM.get().unwrap();
    let irq_guard = trap::disable_local();

    let offset = PLIC_HANDLER.get_with(&irq_guard).get().unwrap().hart_base + CLAIM_OFFSET;
    io_mem.write_once(offset, &(id as u32)).unwrap();
}

pub fn set_priority_threshold_on(cpu: CpuId, threshold: u32) {
    let handler = PLIC_HANDLER.get_on_cpu(cpu).get().unwrap();
    let io_mem = PLIC_IOMEM.get().unwrap();

    let offset = handler.hart_base + THRESHOLD_OFFSET;
    io_mem.write_once(offset, &threshold).unwrap();
}

/// Enable target external interrupt
pub fn enable_external_interrupt(id: u16) {
    // we only enable for cpu 0 for now.
    set_interrupt_enabled_on(CpuId::bsp(), id, true);
    set_priority(id, 1);
    set_priority_threshold_on(CpuId::bsp(), 0);
}

pub(super) fn init() {
    let fdt = DEVICE_TREE.get().unwrap();
    let plic = fdt.find_node("/soc/plic").unwrap();
    let region = plic.reg().unwrap().next().unwrap();
    PLIC_IOMEM.call_once(|| unsafe {
        super::create_device_io_mem(region.starting_address, region.size.unwrap())
    });

    log::debug!("PLIC is found on {:#x?}", region.starting_address);

    let interrupts_ext = plic.property("interrupts-extended").unwrap().value;
    let mut cell_iter = interrupts_ext
        .chunks_exact(4)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()));

    for context_id in 0.. {
        let Some(phandle) = cell_iter.next() else {
            break;
        };
        if phandle == 0 {
            continue;
        }

        let (cpu, intc) = find_cpu_with_intc_phandle(fdt, phandle).unwrap();

        let cell_count = intc.interrupt_cells().unwrap();
        assert!(cell_count >= 1);
        let cause = cell_iter.next().unwrap();
        // consume remaining cells
        for _ in 1..cell_count {
            cell_iter.next().unwrap();
        }

        if cause != Interrupt::SupervisorExternal as u32 {
            continue;
        }

        let hartid = cpu.property("reg").unwrap().as_usize().unwrap();
        log::debug!("Register PLIC context {context_id} on CPU {hartid}, with interrupt {cause}");

        let handler = PLIC_HANDLER.get_on_cpu(hartid.try_into().unwrap());
        handler.call_once(|| PlicHandler {
            context_id,
            hart_base: CONTEXT_BASE + CONTEXT_PER_HART * context_id,
            enable_base: ENABLE_BASE + ENABLE_PER_HART * context_id,
        });
    }

    unsafe {
        riscv::register::sie::set_sext();
    }
}

fn find_cpu_with_intc_phandle<'b, 'a: 'b>(
    fdt: &'b Fdt<'a>,
    phandle: u32,
) -> Option<(FdtNode<'b, 'a>, FdtNode<'b, 'a>)> {
    for cpu in fdt.find_all_nodes("/cpus/cpu") {
        if let Some(intc) = cpu.children().find(|node| {
            node.compatible()
                .is_some_and(|compat| compat.all().any(|c| c == "riscv,cpu-intc"))
        }) && intc
            .property("phandle")
            .is_some_and(|p| p.as_usize().unwrap() as u32 == phandle)
        {
            return Some((cpu, intc));
        }
    }
    None
}
