// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the RISC-V platform.

#![expect(dead_code)]

pub mod boot;
pub(crate) mod cpu;
pub mod device;
mod io;
pub(crate) mod iommu;
pub mod irq;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub(crate) mod serial;
pub(crate) mod task;
pub mod timer;
pub mod trap;

use core::sync::atomic::Ordering;

use crate::{
    arch::{irq::IRQ_CHIP, timer::TIMER_IRQ_NUM},
    cpu::CpuId,
};

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op
}

/// Architecture-specific initialization on the bootstrapping processor.
///
/// # Safety
///
/// 1. This function should only be called once in the boot context of the BSP.
/// 2. This function should be called after the heap allocator is initialized.
/// 3. This function should be called after the kernel page table is activated
///    on the BSP.
pub(crate) unsafe fn late_init_on_bsp() {
    // SAFETY: This function is called in the boot context of the BSP.
    unsafe { trap::init() };

    let io_mem_builder = io::construct_io_mem_allocator_builder();

    // SAFETY:
    // 1. This function is called once and only once in the boot context.
    // 2. No other functions from the `irq` module have been called before this.
    // 3. The kernel page table is already activated.
    unsafe { irq::init(&io_mem_builder) };

    // SAFETY:
    // 1. The caller ensures that the function is only called once in the
    //    boot context of the BSP.
    // 2. The caller ensures that the function is called after the kernel
    //    page table is activated on the BSP.
    unsafe { crate::boot::smp::boot_all_aps() };

    // SAFETY: This function is called once and at most once at a proper timing
    // in the boot context of the BSP, with no timer-related operations having
    // been performed.
    unsafe { timer::init() };

    // SAFETY:
    // 1. All the system device memory have been removed from the builder.
    // 2. RISC-V platforms do not have port I/O.
    unsafe { crate::io::init(io_mem_builder) };

    pci::init();
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    // TODO: We should check for software interrupts too here. Only those external
    // interrupts would go through the IRQ chip.
    if irq_number == TIMER_IRQ_NUM.load(Ordering::Relaxed) as usize {
        return;
    }

    IRQ_CHIP
        .get()
        .unwrap()
        .lock()
        .complete_interrupt(CpuId::current_racy().as_usize() as u32, irq_number as u32);
}

/// Return the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    timer::get_timebase_freq()
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    riscv::register::time::read64()
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns None if no random value was generated.
pub fn read_random() -> Option<u64> {
    // FIXME: Implement a hardware random number generator on RISC-V platforms.
    None
}

pub(crate) fn enable_cpu_features() {
    cpu::extension::init();
    unsafe {
        // We adopt a lazy approach to enable the floating-point unit; it's not
        // enabled before the first FPU trap.
        riscv::register::sstatus::set_fs(riscv::register::sstatus::FS::Off);
    }
}
