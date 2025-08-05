// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the LoongArch platform.

pub mod boot;
pub(crate) mod cpu;
pub mod device;
mod io;
pub(crate) mod iommu;
pub(crate) mod irq;
pub mod kernel;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub(crate) mod serial;
pub(crate) mod task;
pub mod timer;
pub mod trap;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op
}

pub(crate) unsafe fn late_init_on_bsp() {
    // SAFETY: This function is called in the boot context of the BSP.
    unsafe { trap::init() };

    let io_mem_builder = io::construct_io_mem_allocator_builder();

    kernel::irq::init();

    // SAFETY: We're on the BSP and we're ready to boot all APs.
    unsafe { crate::boot::smp::boot_all_aps() };

    // SAFETY:
    // 1. All the system device memory have been removed from the builder.
    // 2. LoongArch platforms do not have port I/O.
    unsafe { crate::io::init(io_mem_builder) };

    pci::init();
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
