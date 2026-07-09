// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the AArch64 platform.

#![expect(dead_code)]

pub mod boot;
pub mod cpu;
pub mod device;
pub(crate) mod io;
pub(crate) mod iommu;
pub mod irq;
pub(crate) mod mm;
mod power;
pub mod serial;
pub(crate) mod task;
mod timer;
pub mod trap;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op.
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
    // SAFETY: This is only called once on this BSP in the boot context.
    unsafe { trap::init_on_cpu() };

    // SAFETY: The caller ensures that this function is only called once on the
    // BSP, after the kernel page table is activated.
    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    // SAFETY: This function is called once at a proper timing in the boot
    // context of the BSP.
    unsafe { irq::chip::init_on_bsp() };

    // SAFETY: This is called on the BSP before any IPI-related operation.
    unsafe { irq::ipi::init_on_bsp() };

    // SAFETY: This function is called once at a proper timing in the boot
    // context of the BSP.
    unsafe { timer::init_on_bsp() };

    // SAFETY: We're on the BSP and we're ready to boot all APs.
    unsafe { crate::boot::smp::boot_all_aps() };

    // SAFETY:
    // 1. All the system device memory has been removed from the builder.
    // 2. AArch64 platforms do not have port I/O.
    unsafe { crate::io::init(io_mem_builder) };

    power::init();
}

/// Initializes application-processor-specific state.
///
/// # Safety
///
/// 1. This function must be called only once on each application processor.
/// 2. This function must be called after the BSP's call to [`late_init_on_bsp`]
///    and before any other architecture-specific code in this module is called
///    on this AP.
pub(crate) unsafe fn init_on_ap() {
    // SAFETY: The safety is upheld by the caller.
    unsafe { trap::init_on_cpu() };

    // SAFETY: The safety is upheld by the caller.
    unsafe { irq::chip::init_on_ap() };

    // SAFETY: The caller ensures that this is only called once on this AP.
    unsafe { timer::init_on_ap() };
}

/// Returns the frequency of the system counter. The unit is Hz.
pub fn tsc_freq() -> u64 {
    timer::get_timebase_freq()
}

/// Reads the current value of the processor's system counter (`CNTPCT_EL0`).
pub fn read_tsc() -> u64 {
    let cnt: u64;
    // SAFETY: Reading `CNTPCT_EL0` has no side effects.
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nostack, nomem)) };
    cnt
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns `None` if no random value was generated.
///
/// TODO: Use the Armv8.5 `RNDR` register when the `FEAT_RNG` feature is present.
pub fn read_random() -> Option<u64> {
    None
}

pub(crate) fn enable_cpu_features() {
    cpu::extension::init();
}
