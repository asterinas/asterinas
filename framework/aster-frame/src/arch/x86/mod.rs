// SPDX-License-Identifier: MPL-2.0

pub mod boot;
pub mod console;
pub(crate) mod cpu;
pub mod device;
pub mod iommu;
pub(crate) mod irq;
pub(crate) mod kernel;
pub(crate) mod mm;
pub(crate) mod pci;
pub mod qemu;
pub mod smp;
#[cfg(feature = "intel_tdx")]
pub(crate) mod tdx_guest;
pub(crate) mod timer;

use core::{arch::x86_64::_rdtsc, sync::atomic::Ordering};

#[cfg(feature = "intel_tdx")]
use ::tdx_guest::tdx_is_enabled;
use kernel::apic::ioapic;
use log::{info, warn};

use self::irq::enable_local;

pub(crate) fn before_all_init() {
    enable_common_cpu_features();
    console::init();
}

pub(crate) fn after_all_init() {
    irq::init();
    mm::init();
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
    console::callback_init();
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
    // TODO: Implement LAPIC configuration capabilities for local CPU interrupt priority management
    // and interrupt broadcasting across multiple cores. This enhancement will address the current
    // limitation where the IOAPIC forwards all interrupts exclusively to CPU 0, necessitating that all
    // interrupts are bound to core 0 by default.
    // The present early activation of interrupts is a temporary measure to mitigate the issue of delayed
    // interrupt enabling, which currently occurs at user-space program initiation—potentially on other cores.
    enable_local();
}

pub(crate) fn interrupts_ack() {
    kernel::pic::ack();
    if kernel::apic::APIC_TYPE.is_completed() {
        kernel::apic::APIC_INSTANCE.borrow().eoi();
    }
}

/// Return the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    kernel::tsc::TSC_FREQ.load(Ordering::Acquire)
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    // Safety: It is safe to read a time-related counter.
    unsafe { _rdtsc() }
}

pub(crate) fn enable_common_cpu_features() {
    use x86_64::registers::{control::Cr4Flags, model_specific::EferFlags, xcontrol::XCr0Flags};
    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::FSGSBASE | Cr4Flags::OSXSAVE | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE;
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
