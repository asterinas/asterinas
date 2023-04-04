use alloc::vec::Vec;
use spin::{Mutex, Once};

use crate::{trap::IrqLine, util::recycle_allocator::RecycleAllocator};

/// The IRQ numbers which are not using
pub(crate) static NOT_USING_IRQ: Mutex<RecycleAllocator> =
    Mutex::new(RecycleAllocator::with_start_max(32, 256));

pub(crate) static IRQ_LIST: Once<Vec<IrqLine>> = Once::new();

pub(crate) fn init() {
    let mut list: Vec<IrqLine> = Vec::new();
    for i in 0..256 {
        list.push(IrqLine {
            irq_num: i as u8,
            callback_list: Mutex::new(Vec::new()),
        });
    }
    IRQ_LIST.call_once(|| list);
}

pub(crate) fn enable_local() {
    x86_64::instructions::interrupts::enable();
}

pub(crate) fn disable_local() {
    x86_64::instructions::interrupts::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    x86_64::instructions::interrupts::are_enabled()
}
