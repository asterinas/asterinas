// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the x86 platform.

pub mod boot;
pub(crate) mod cpu;
pub mod device;
pub(crate) mod ex_table;
pub mod iommu;
pub(crate) mod irq;
pub(crate) mod kernel;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub mod serial;
pub mod task;
#[cfg(feature = "intel_tdx")]
pub(crate) mod tdx_guest;
pub mod timer;
pub mod trap;

use core::{
    arch::x86_64::{_rdrand64_step, _rdtsc},
    sync::atomic::Ordering,
};

#[cfg(feature = "intel_tdx")]
use ::tdx_guest::tdx_is_enabled;
use kernel::apic::ioapic;
use log::{info, warn};

pub(crate) fn before_all_init() {
    enable_common_cpu_features();
    serial::init();
}

pub(crate) fn after_all_init() {
    irq::init();
    kernel::acpi::init();
    match kernel::apic::init() {
        Ok(_) => {
            ioapic::init();
        }
        Err(err) => {
            info!("APIC init error:{:?}", err);
            kernel::pic::enable();
        }
    }
    serial::callback_init();
    timer::init();
    #[cfg(feature = "intel_tdx")]
    if !tdx_is_enabled() {
        match iommu::init() {
            Ok(_) => {}
            Err(err) => warn!("IOMMU initialization error:{:?}", err),
        }
    }
    #[cfg(not(feature = "intel_tdx"))]
    match iommu::init() {
        Ok(_) => {}
        Err(err) => warn!("IOMMU initialization error:{:?}", err),
    }
    // Some driver like serial may use PIC
    kernel::pic::init();
}

pub(crate) fn interrupts_ack() {
    kernel::pic::ack();
    if let Some(apic) = kernel::apic::APIC_INSTANCE.get() {
        apic.lock_irq_disabled().eoi();
    }
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    kernel::tsc::TSC_FREQ.load(Ordering::Acquire)
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    // SAFETY: It is safe to read a time-related counter.
    unsafe { _rdtsc() }
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns None if no random value was generated.
pub fn read_random() -> Option<u64> {
    // Recommendation from "Intel® Digital Random Number Generator (DRNG) Software
    // Implementation Guide" - Section 5.2.1 and "Intel® 64 and IA-32 Architectures
    // Software Developer’s Manual" - Volume 1 - Section 7.3.17.1.
    const RETRY_LIMIT: usize = 10;

    for _ in 0..RETRY_LIMIT {
        let mut val = 0;
        let generated = unsafe { _rdrand64_step(&mut val) };
        if generated == 1 {
            return Some(val);
        }
    }
    None
}

fn enable_common_cpu_features() {
    use x86_64::registers::{control::Cr4Flags, model_specific::EferFlags, xcontrol::XCr0Flags};
    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::FSGSBASE
        | Cr4Flags::OSXSAVE
        | Cr4Flags::OSFXSR
        | Cr4Flags::OSXMMEXCPT_ENABLE
        | Cr4Flags::PAGE_GLOBAL;
    unsafe {
        x86_64::registers::control::Cr4::write(cr4);
    }

    let mut xcr0 = x86_64::registers::xcontrol::XCr0::read();
    xcr0 |= XCr0Flags::AVX | XCr0Flags::SSE;
    unsafe {
        x86_64::registers::xcontrol::XCr0::write(xcr0);
    }

    unsafe {
        // enable non-executable page protection
        x86_64::registers::model_specific::Efer::update(|efer| {
            *efer |= EferFlags::NO_EXECUTE_ENABLE;
        });
    }
}
