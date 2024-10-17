// SPDX-License-Identifier: MPL-2.0

//! PLIC

use core::ops::Range;

use bit_field::BitField;
use spin::Once;

use crate::{arch::boot::DEVICE_TREE, early_println, io_mem::IoMem, mm::VmIoOnce};

struct Plic {
    io_mem: IoMem,
}

impl Plic {
    fn set_priority(&self, id: u16, priority: u32) {
        self.io_mem
            .write_once(4 * (id as usize), &priority)
            .unwrap();
    }

    fn set_interrupt_enabled(&self, context: u32, id: u16, enabled: bool) {
        let offset = 0x2000 + context as usize * 0x80 + (id as usize / 32) * 4;

        let mut value: u32 = self.io_mem.read_once(offset).unwrap();
        value.set_bit((id % 32) as usize, enabled);
        self.io_mem.write_once(offset, &value).unwrap();
    }

    fn complete_interrupt(&self, context: u32, id: u16) {
        let offset = 0x200004 + context * 0x1000;
        self.io_mem
            .write_once(offset as usize, &(id as u32))
            .unwrap();
    }

    fn claim_interrupt(&self, context: u32) -> u16 {
        let offset = 0x200004 + context * 0x1000;
        self.io_mem.read_once::<u32>(offset as usize).unwrap() as u16
    }

    unsafe fn new(range: Range<usize>) -> Self {
        let plic = Self {
            io_mem: IoMem::new(range),
        };

        const CONTEXT_LIMIT: u32 = 15872;
        const INTERRUPT_LIMIT: u16 = 1024;
        for id in 0..INTERRUPT_LIMIT {
            plic.set_priority(id, 0);
        }

        for context in 0..CONTEXT_LIMIT {
            for id in 0..INTERRUPT_LIMIT {
                plic.set_interrupt_enabled(context, id, false);
            }
        }

        plic
    }
}

static PLIC: Once<Plic> = Once::new();

pub(crate) fn init() {
    for child in DEVICE_TREE
        .get()
        .unwrap()
        .find_node("/soc")
        .unwrap()
        .children()
    {
        early_println!("child name:{:?}", child.name);
    }
    let plic = DEVICE_TREE.get().unwrap().find_node("/soc/plic").unwrap();

    let reg = plic.reg().unwrap();
    for region in reg {
        PLIC.call_once(|| unsafe {
            Plic::new(
                region.starting_address as usize
                    ..(region.starting_address as usize + region.size.unwrap()),
            )
        });
    }
}

/// Enable target external interrupt
pub fn enable_external_interrupt(irq: u16, priority: u32) {
    // FIXME: we only access context 0 for now.
    if let Some(plic) = PLIC.get() {
        plic.set_interrupt_enabled(0, irq, true);
        plic.set_priority(irq, priority);
    }
}

pub(crate) fn claim_interrupt() -> u16 {
    // FIXME: we only access context 0 for now.
    if let Some(plic) = PLIC.get() {
        plic.claim_interrupt(0)
    } else {
        0
    }
}

pub(crate) fn complete_interrupt(irq: u16) {
    // FIXME: we only access context 0 for now.
    if let Some(plic) = PLIC.get() {
        plic.complete_interrupt(0, irq);
    }
}
