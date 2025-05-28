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

use irq::IRQ_CHIP;

use crate::cpu::CpuId;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op
}

/// Architecture-specific initialization on the bootstrapping processor after
/// heap and frame allocators are initialized.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function should be called after the heap and frame allocators are
///    initialized.
pub(crate) unsafe fn init_on_bsp_after_heap() {
    // SAFETY: This function is called in the boot context of the BSP.
    unsafe { trap::init() };

    let io_mem_builder = io::construct_io_mem_allocator_builder();

    // SAFETY: This function is called once and at most once here at a proper timing
    // in the boot context of the BSP, with no irq-related operations having
    // been performed.
    unsafe { irq::init(&io_mem_builder) };

    // SAFETY: We're on the BSP and we're ready to boot all APs.
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

/// Architecture-specific initialization on the bootstrapping processor after
/// kernel page table is activated.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function should be called after the kernel page table is activated.
pub(crate) unsafe fn init_on_bsp_after_kpt() {
    // SAFETY: This function is called only once in the boot context of the BSP,
    // after the kernel page table is activated.
    unsafe { irq::init_after_kpt() };
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    // Invoked always in interrupt context, so there's no race condition.
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
