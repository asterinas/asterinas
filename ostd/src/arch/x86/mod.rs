// SPDX-License-Identifier: MPL-2.0

//! Platform-specific code for the x86 platform.

pub mod boot;
pub(crate) mod cpu;
pub mod device;
pub(crate) mod ex_table;
pub(crate) mod io;
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

use io::construct_io_mem_allocator_builder;
use spin::Once;
use x86::cpuid::{CpuId, FeatureInfo};

#[cfg(feature = "cvm_guest")]
pub(crate) mod tdx_guest;

use core::sync::atomic::Ordering;

use kernel::apic::ioapic;
pub use kernel::IO_APIC;
use log::{info, warn};

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {
    match ::tdx_guest::init_tdx() {
        Ok(td_info) => {
            crate::early_println!(
                "[kernel] Intel TDX initialized\n[kernel] td gpaw: {}, td attributes: {:?}",
                td_info.gpaw,
                td_info.attributes
            );
        }
        Err(::tdx_guest::tdcall::InitError::TdxGetVpInfoError(td_call_error)) => {
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

/// Architecture-specific initialization on the bootstrapping processor.
///
/// It should be called when the heap and frame allocators are available.
///
/// # Safety
///
/// This function must be called only once in the boot context of the
/// bootstrapping processor.
pub(crate) unsafe fn late_init_on_bsp() {
    // SAFETY: This function is only called once on BSP.
    unsafe { trap::init() };

    let io_mem_builder = construct_io_mem_allocator_builder();

    kernel::apic::init(&io_mem_builder).expect("APIC doesn't exist");
    kernel::irq::init(&io_mem_builder);

    kernel::tsc::init_tsc_freq();
    timer::init_bsp();

    // SAFETY: We're on the BSP and we're ready to boot all APs.
    unsafe { crate::boot::smp::boot_all_aps() };

    if_tdx_enabled!({
    } else {
        match iommu::init(&io_mem_builder) {
            Ok(_) => {}
            Err(err) => warn!("IOMMU initialization error:{:?}", err),
        }
    });

    // SAFETY:
    // 1. All the system device memory have been removed from the builder.
    // 2. All the port I/O regions belonging to the system device are defined using the macros.
    // 3. `MAX_IO_PORT` defined in `crate::arch::io` is the maximum value specified by x86-64.
    unsafe { crate::io::init(io_mem_builder) };
}

/// Architecture-specific initialization on the application processor.
///
/// # Safety
///
/// This function must be called only once on each application processor.
/// And it should be called after the BSP's call to [`init_on_bsp`].
///
/// [`init_on_bsp`]: crate::cpu::init_on_bsp
pub(crate) unsafe fn init_on_ap() {
    timer::init_ap();
}

pub(crate) fn interrupts_ack(irq_number: usize) {
    if !cpu::context::CpuException::is_cpu_exception(irq_number) {
        // TODO: We're in the interrupt context, so `disable_preempt()` is not
        // really necessary here.
        kernel::apic::get_or_init(&crate::task::disable_preempt() as _).eoi();
    }
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    kernel::tsc::TSC_FREQ.load(Ordering::Acquire)
}

/// Reads the current value of the processor’s time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    use core::arch::x86_64::_rdtsc;

    // SAFETY: It is safe to read a time-related counter.
    unsafe { _rdtsc() }
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns None if no random value was generated.
pub fn read_random() -> Option<u64> {
    use core::arch::x86_64::_rdrand64_step;

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

fn has_avx() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};

    let cpuid_result = unsafe { __cpuid(0) };
    if cpuid_result.eax < 1 {
        // CPUID function 1 is not supported
        return false;
    }

    let cpuid_result = unsafe { __cpuid_count(1, 0) };
    // Check for AVX (bit 28 of ecx)
    cpuid_result.ecx & (1 << 28) != 0
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

    xcr0 |= XCr0Flags::SSE;

    if has_avx() {
        xcr0 |= XCr0Flags::AVX;
    }

    if has_avx512() {
        xcr0 |= XCr0Flags::OPMASK | XCr0Flags::ZMM_HI256 | XCr0Flags::HI16_ZMM;
    }

    unsafe {
        x86_64::registers::xcontrol::XCr0::write(xcr0);
    }

    cpu::context::enable_essential_features();

    unsafe {
        // enable non-executable page protection
        x86_64::registers::model_specific::Efer::update(|efer| {
            *efer |= EferFlags::NO_EXECUTE_ENABLE;
        });
    }
}

/// Inserts a TDX-specific code block.
///
/// This macro conditionally executes a TDX-specific code block based on the following conditions:
/// (1) The `cvm_guest` feature is enabled at compile time.
/// (2) The TDX feature is detected at runtime via `::tdx_guest::tdx_is_enabled()`.
///
/// If both conditions are met, the `if_block` is executed. If an `else_block` is provided, it will be executed
/// when either the `cvm_guest` feature is not enabled or the TDX feature is not detected at runtime.
#[macro_export]
macro_rules! if_tdx_enabled {
    // Match when there is an else block
    ($if_block:block else $else_block:block) => {{
        #[cfg(feature = "cvm_guest")]
        {
            if ::tdx_guest::tdx_is_enabled() {
                $if_block
            } else {
                $else_block
            }
        }
        #[cfg(not(feature = "cvm_guest"))]
        {
            $else_block
        }
    }};
    // Match when there is no else block
    ($if_block:block) => {{
        #[cfg(feature = "cvm_guest")]
        {
            if ::tdx_guest::tdx_is_enabled() {
                $if_block
            }
        }
    }};
}

pub use if_tdx_enabled;
