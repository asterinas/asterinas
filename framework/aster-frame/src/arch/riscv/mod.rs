// SPDX-License-Identifier: MPL-2.0

pub mod boot;
pub mod console;
pub(crate) mod cpu;
pub mod device;
pub mod iommu;
pub(crate) mod irq;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub mod timer;
pub mod task;
pub mod trap;

use core::sync::atomic::Ordering;

use log::warn;

pub(crate) fn before_all_init() {
    console::init();
}

pub(crate) fn after_all_init() {
    irq::init();
    timer::init();
    match iommu::init() {
        Ok(_) => {}
        Err(err) => warn!("IOMMU initialization error:{:?}", err),
    }
}

pub(crate) fn interrupts_ack() {
    todo!()
}

/// Return the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    timer::TIMEBASE_FREQ.load(Ordering::Relaxed)
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    riscv::register::time::read64()
}
