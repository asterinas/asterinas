// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use spin::Once;

use crate::{sync::SpinLock, trap::irq::SystemIrqLine, util::recycle_allocator::RecycleAllocator};

pub(crate) fn enable_local() {
    riscv::interrupt::supervisor::enable();
}

pub(crate) fn disable_local() {
    unsafe {
        riscv::interrupt::supervisor::disable();
    }
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
}

pub(crate) static IRQ_NUM_ALLOCATOR: SpinLock<RecycleAllocator> =
    SpinLock::new(RecycleAllocator::with_start_max(32, 256));

pub(crate) static IRQ_LIST: Once<Vec<SystemIrqLine>> = Once::new();

pub(crate) fn init() {
    let mut list: Vec<SystemIrqLine> = Vec::new();
    for i in 0..256 {
        list.push(SystemIrqLine {
            irq_num: i as u8,
            callback_list: SpinLock::new(Vec::new()),
        });
    }
    IRQ_LIST.call_once(|| list);
}
