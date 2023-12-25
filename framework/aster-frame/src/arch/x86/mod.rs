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
#[cfg(feature = "intel_tdx")]
pub(crate) mod tdx_guest;
pub(crate) mod timer;

use core::{arch::x86_64::_rdtsc, sync::atomic::Ordering};

use kernel::apic::ioapic;
use log::{info, warn};

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
        apic.lock().eoi();
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

fn enable_common_cpu_features() {
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
