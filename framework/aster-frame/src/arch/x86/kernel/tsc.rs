use core::sync::atomic::AtomicU64;
use x86::cpuid::cpuid;

/// The frequency of tsc. The unit is Hz.
pub(crate) static TSC_FREQ: AtomicU64 = AtomicU64::new(0);

const TSC_DEADLINE_MODE_SUPPORT: u32 = 1 << 24;

/// Determine if the current system supports tsc_deadline mode.
pub fn is_tsc_deadline_mode_supported() -> bool {
    let cpuid = cpuid!(1);
    (cpuid.ecx & TSC_DEADLINE_MODE_SUPPORT) > 0
}

/// Determine TSC frequency via CPUID. If the CPU does not support calculating TSC frequency by
/// CPUID, the function will return None. The unit of the return value is KHz.
///
/// Ref: function `native_calibrate_tsc` in linux `arch/x86/kernel/tsc.c`
///
pub fn determine_tsc_freq_via_cpuid() -> Option<u32> {
    // Check the max cpuid supported
    let cpuid = cpuid!(0);
    let max_cpuid = cpuid.eax;
    if max_cpuid <= 0x15 {
        return None;
    }

    // TSC frequecny = ecx * ebx / eax
    // CPUID 0x15: Time Stamp Counter and Nominal Core Crystal Clock Information Leaf
    let mut cpuid = cpuid!(0x15);
    if cpuid.eax == 0 || cpuid.ebx == 0 {
        return None;
    }
    let eax_denominator = cpuid.eax;
    let ebx_numerator = cpuid.ebx;
    let mut crystal_khz = cpuid.ecx / 1000;

    // Some Intel SoCs like Skylake and Kabylake don't report the crystal
    // clock, but we can easily calculate it to a high degree of accuracy
    // by considering the crystal ratio and the CPU speed.
    if crystal_khz == 0 && max_cpuid >= 0x16 {
        cpuid = cpuid!(0x16);
        let base_mhz = cpuid.eax;
        crystal_khz = base_mhz * 1000 * eax_denominator / ebx_numerator;
    }

    if crystal_khz == 0 {
        None
    } else {
        Some(crystal_khz * ebx_numerator / eax_denominator)
    }
}
