// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the RISC-V platform.

pub mod boot;
pub(crate) mod cpu;
pub mod device;
pub mod iommu;
pub(crate) mod irq;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub mod serial;
pub mod task;
pub mod timer;
pub mod trap;

use core::sync::atomic::Ordering;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    // Unimplemented, no-op
}

pub(crate) fn init_on_bsp() {
    // SAFETY: this function is only called once on BSP.
    unsafe {
        trap::init(true);
    }
    irq::init();

    // SAFETY: they are only called once on BSP and ACPI has been initialized.
    unsafe {
        crate::cpu::init_num_cpus();
        crate::cpu::set_this_cpu_id(0);
    }

    // SAFETY: no CPU local objects have been accessed by this far. And
    // we are on the BSP.
    unsafe { crate::cpu::local::init_on_bsp() };

    crate::boot::smp::boot_all_aps();

    timer::init();
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    unimplemented!()
}

/// Return the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    timer::TIMEBASE_FREQ.load(Ordering::Relaxed)
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    riscv::register::time::read64()
}

pub(crate) fn enable_cpu_features() {
    unsafe {
        riscv::register::sstatus::set_fs(riscv::register::sstatus::FS::Clean);
    }
}
