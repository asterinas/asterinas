// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the ARM platform.

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
    // SAFETY: This is only called once on this BSP in the boot context.
    unsafe { trap::init_on_cpu() };

    // SAFETY: The caller ensures that this function is only called once on BSP,
    // after the kernel page table is activated.
    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    // SAFETY: We're on the BSP and we're ready to boot all APs.
    unsafe { crate::boot::smp::boot_all_aps() };

    // SAFETY:
    // 1. All the system device memory have been removed from the builder.
    // 2. ARM platforms do not have port I/O.
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
    unimplemented!()
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    use core::arch::asm;

    let cntfrq;
    // SAFETY: It is safe to read a time-related counter.
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) cntfrq) };
    cntfrq
}

/// Reads the current value of the processor's time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    use core::arch::asm;

    let cntvct;
    // SAFETY: It is safe to read a time-related counter.
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) cntvct) };
    cntvct
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns `None` if no random value was generated.
pub fn read_random() -> Option<u64> {
    // FIXME: Implement a hardware random number generator on ARM platforms.
    None
}

pub(crate) fn enable_cpu_features() {
    use core::arch::asm;

    // SAFETY: It is safe to enable access to the FPU, as the FPU state does not
    // affect the kernel's memory safety.
    unsafe {
        // Architectural Feature Access Control Register (CPACR).
        // FPEN, bits [21:20] = 11: Instructions that use the registers associated
        // with Advanced SIMD and floating-point execution can be used at EL0/EL1.
        asm!(
            "mov {tmp}, #(3 << 20)",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
        );
    }
}
