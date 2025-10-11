// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the LoongArch platform.

#![expect(dead_code)]

pub mod boot;
pub mod cpu;
pub mod device;
mod io;
pub(crate) mod iommu;
pub(crate) mod irq;
pub mod kernel;
pub(crate) mod mm;
pub mod qemu;
pub(crate) mod serial;
pub(crate) mod task;
mod timer;
pub mod trap;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op
}

/// Architecture-specific initialization on the bootstrapping processor.
///
/// It should be called when the heap and frame allocators are available.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function must be called after the kernel page table is activated on
///    the bootstrapping processor.
pub(crate) unsafe fn late_init_on_bsp() {
    // SAFETY: This function is called in the boot context of the BSP.
    unsafe { trap::init() };

    // SAFETY: The caller ensures that this function is only called once on BSP,
    // after the kernel page table is activated.
    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    kernel::irq::init();

    // SAFETY: We're on the BSP and we're ready to boot all APs.
    unsafe { crate::boot::smp::boot_all_aps() };

    // SAFETY:
    // 1. All the system device memory have been removed from the builder.
    // 2. LoongArch platforms do not have port I/O.
    unsafe { crate::io::init(io_mem_builder) };
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    kernel::irq::complete(irq_number as _);
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    loongArch64::time::get_timer_freq() as _
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    loongArch64::time::Time::read() as _
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns None if no random value was generated.
pub fn read_random() -> Option<u64> {
    // FIXME: Implement a hardware random number generator on LoongArch platforms.
    None
}

pub(crate) fn enable_cpu_features() {}
