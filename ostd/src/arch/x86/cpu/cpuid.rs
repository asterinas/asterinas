// SPDX-License-Identifier: MPL-2.0

//! CPU information from the CPUID instruction.

use core::arch::x86_64::CpuidResult;

use spin::Once;

static MAX_LEAF: Once<u32> = Once::new();
static MAX_EXTENDED_LEAF: Once<u32> = Once::new();

#[repr(u32)]
enum Leaf {
    Base = 0x00,
    Xstate = 0x0d,
    Tsc = 0x15,

    ExtBase = 0x80000000,
}

/// Executes the CPUID instruction for the given leaf and subleaf.
///
/// This method will return `None` if the leaf is not supported.
pub fn cpuid(leaf: u32, subleaf: u32) -> Option<CpuidResult> {
    fn raw_cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
        // SAFETY: It is safe to execute the CPUID instruction.
        unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) }
    }

    let max_leaf = if leaf < Leaf::ExtBase as u32 {
        *MAX_LEAF.call_once(|| raw_cpuid(Leaf::Base as u32, 0).eax)
    } else {
        *MAX_EXTENDED_LEAF.call_once(|| raw_cpuid(Leaf::ExtBase as u32, 0).eax)
    };

    if leaf > max_leaf {
        None
    } else {
        Some(raw_cpuid(leaf, subleaf))
    }
}

/// Queries the frequency in Hz of the Time Stamp Counter (TSC).
///
/// This is based on the information given by the CPUID instruction in the Time Stamp Counter and
/// Nominal Core Crystal Clock Information Leaf.
///
/// Note that the CPUID leaf is currently only supported by new Intel CPUs. This method will return
/// `None` if it is not supported.
pub(in crate::arch) fn query_tsc_freq() -> Option<u64> {
    let CpuidResult {
        eax: denominator,
        ebx: numerator,
        ecx: crystal_freq,
        ..
    } = cpuid(Leaf::Tsc as u32, 0)?;

    if denominator == 0 || numerator == 0 {
        return None;
    }

    // If the nominal core crystal clock frequency is not enumerated, we can either obtain that
    // information from a hardcoded table or rely on the processor base frequency. The Intel
    // documentation recommends the first approach [1], but Linux uses the second approach because
    // the first approach is difficult to implement correctly for all corner cases [2]. However,
    // the second approach does not provide 100% accurate frequencies, so Linux must adjust them at
    // runtime [2]. For now, we avoid these headaches by faithfully reporting that the TSC
    // frequency is unavailable.
    //
    // [1]: Intel(R) 64 and IA-32 Architectures Software Developer’s Manual,
    //      Section 20.7.3, Determining the Processor Base Frequency
    // [2]: https://github.com/torvalds/linux/commit/604dc9170f2435d27da5039a3efd757dceadc684
    if crystal_freq == 0 {
        return None;
    }

    Some((crystal_freq as u64) * (numerator as u64) / (denominator as u64))
}

/// Queries the supported XSTATE features, i.e., the supported bits of `XCR0` and `IA32_XSS`.
pub(in crate::arch) fn query_xstate_max_features() -> Option<u64> {
    let res0 = cpuid(Leaf::Xstate as u32, 0)?;
    let res1 = cpuid(Leaf::Xstate as u32, 1)?;

    // Supported bits in `XCR0`.
    let xcr_bits = (res0.eax as u64) | ((res0.edx as u64) << 32);
    // Supported bits in `IA32_XSS`.
    let xss_bits = (res1.ecx as u64) | ((res1.edx as u64) << 32);

    Some(xcr_bits | xss_bits)
}

/// Queries the size in bytes of the XSAVE area containing states enabled by `XCRO` and `IA32_XSS`.
pub(in crate::arch) fn query_xsave_area_size() -> Option<u32> {
    cpuid(Leaf::Xstate as u32, 1).map(|res| res.ebx)
}
