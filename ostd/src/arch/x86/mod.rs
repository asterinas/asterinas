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
pub mod timer;
pub mod trap;

use cfg_if::cfg_if;
use spin::Once;
use x86::cpuid::{CpuId, FeatureInfo};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        pub(crate) mod tdx_guest;

        use {
            crate::early_println,
            ::tdx_guest::{init_tdx, tdcall::InitError, tdx_is_enabled},
        };
    }
}

use core::{
    arch::x86_64::{_rdrand64_step, _rdtsc},
    sync::atomic::Ordering,
};

use kernel::apic::ioapic;
use log::{info, warn};

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    match init_tdx() {
        Ok(td_info) => {
            early_println!(
                "[kernel] Intel TDX initialized\n[kernel] td gpaw: {}, td attributes: {:?}",
                td_info.gpaw,
                td_info.attributes
            );
        }
        Err(InitError::TdxGetVpInfoError(td_call_error)) => {
            panic!(
                "[kernel] Intel TDX not initialized, Failed to get TD info: {:?}",
                td_call_error
            );
        }
        // The machine has no TDX support.
        Err(_) => {}
    }
}

static CPU_FEATURES: Once<FeatureInfo> = Once::new();

pub(crate) fn init_on_bsp() {
    // SAFETY: this function is only called once on BSP.
    unsafe {
        crate::arch::trap::init(true);
    }
    irq::init();
    kernel::acpi::init();

    // SAFETY: they are only called once on BSP and ACPI has been initialized.
    unsafe {
        crate::cpu::init_num_cpus();
        crate::cpu::set_this_cpu_id(0);
    }

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

    // SAFETY: no CPU local objects have been accessed by this far. And
    // we are on the BSP.
    unsafe { crate::cpu::local::init_on_bsp() };

    crate::boot::smp::boot_all_aps();

    timer::init();

    cfg_if! {
        if #[cfg(feature = "cvm_guest")] {
            if !tdx_is_enabled() {
                match iommu::init() {
                    Ok(_) => {}
                    Err(err) => warn!("IOMMU initialization error:{:?}", err),
                }
            }
        } else {
            match iommu::init() {
                Ok(_) => {}
                Err(err) => warn!("IOMMU initialization error:{:?}", err),
            }
        }
    }

    // Some driver like serial may use PIC
    kernel::pic::init();
}

/// Architecture-specific initialization on the application processor.
///
/// # Safety
///
/// This function must be called only once on each application processor.
/// And it should be called after the BSP's call to [`init_on_bsp`].
pub(crate) unsafe fn init_on_ap() {
    // Trigger the initialization of the local APIC.
    crate::arch::x86::kernel::apic::with_borrow(|_| {});
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    if !cpu::CpuException::is_cpu_exception(irq_number as u16) {
        kernel::pic::ack();
        kernel::apic::with_borrow(|apic| {
            apic.eoi();
        });
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

fn has_avx512() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};

    let cpuid_result = unsafe { __cpuid(0) };
    if cpuid_result.eax < 7 {
        // CPUID function 7 is not supported
        return false;
    }

    let cpuid_result = unsafe { __cpuid_count(7, 0) };
    // Check for AVX-512 Foundation (bit 16 of ebx)
    cpuid_result.ebx & (1 << 16) != 0
}

pub(crate) fn enable_cpu_features() {
    use x86_64::registers::{control::Cr4Flags, model_specific::EferFlags, xcontrol::XCr0Flags};

    CPU_FEATURES.call_once(|| {
        let cpuid = CpuId::new();
        cpuid.get_feature_info().unwrap()
    });

    cpu::enable_essential_features();

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

    if has_avx512() {
        xcr0 |= XCr0Flags::OPMASK | XCr0Flags::ZMM_HI256 | XCr0Flags::HI16_ZMM;
    }

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
