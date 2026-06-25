// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{Ordering, compiler_fence};

use spin::Once;
use x86::msr;

use crate::mm::{Frame, FrameAllocOptions, HasPaddr, paddr_to_vaddr};

// KVM CPUID leaves, signature, feature bits, and MSR values.
// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/include/uapi/asm/kvm_para.h>.
const KVM_CPUID_SIGNATURE: u32 = 0x4000_0000;
const KVM_CPUID_FEATURES: u32 = 0x4000_0001;
const KVM_SIGNATURE: &[u8; 12] = b"KVMKVMKVM\0\0\0";
const KVM_FEATURE_CLOCKSOURCE2: u32 = 3;
const MSR_KVM_SYSTEM_TIME_NEW: u32 = 0x4b56_4d01;
const KVM_MSR_ENABLED: u64 = 1;

static PVCLOCK_FRAME: Once<Option<Frame<()>>> = Once::new();

#[repr(C, align(4))]
struct PvclockVcpuTimeInfo {
    version: u32,
    _pad0: u32,
    _tsc_timestamp: u64,
    _system_time: u64,
    tsc_to_system_mul: u32,
    tsc_shift: i8,
    _flags: u8,
    _pad: [u8; 2],
}

struct PvclockTimeSnapshot {
    tsc_to_system_mul: u32,
    tsc_shift: i8,
}

/// Determines the TSC frequency from KVM's paravirtual clock.
pub(super) fn determine_tsc_freq() -> Option<u64> {
    if !has_kvm_clocksource2() {
        return None;
    }

    let pvclock_info = enable_system_time_msr()?;
    let time_info = read_stable_time_info(pvclock_info)?;
    freq_from_time_info(&time_info)
}

fn has_kvm_clocksource2() -> bool {
    let Some(signature_leaf) = crate::arch::cpu::cpuid::cpuid(KVM_CPUID_SIGNATURE, 0) else {
        return false;
    };

    let mut signature = [0u8; 12];
    signature[0..4].copy_from_slice(&signature_leaf.ebx.to_le_bytes());
    signature[4..8].copy_from_slice(&signature_leaf.ecx.to_le_bytes());
    signature[8..12].copy_from_slice(&signature_leaf.edx.to_le_bytes());
    if &signature != KVM_SIGNATURE {
        return false;
    }

    let Some(features) = crate::arch::cpu::cpuid::cpuid(KVM_CPUID_FEATURES, 0) else {
        return false;
    };
    features.eax & (1 << KVM_FEATURE_CLOCKSOURCE2) != 0
}

fn enable_system_time_msr() -> Option<*const PvclockVcpuTimeInfo> {
    let frame = PVCLOCK_FRAME.call_once(|| FrameAllocOptions::new().alloc_frame().ok());
    let frame = frame.as_ref()?;
    let paddr = frame.paddr();

    // SAFETY: `paddr` is the physical address of a live, zeroed page retained by
    // `PVCLOCK_FRAME`. KVM owns updates to the `PvclockVcpuTimeInfo` fields after
    // this MSR is enabled.
    unsafe {
        msr::wrmsr(MSR_KVM_SYSTEM_TIME_NEW, paddr as u64 | KVM_MSR_ENABLED);
        Some(paddr_to_vaddr(paddr) as *const PvclockVcpuTimeInfo)
    }
}

fn read_stable_time_info(pvclock_info: *const PvclockVcpuTimeInfo) -> Option<PvclockTimeSnapshot> {
    const MAX_RETRIES: usize = 1_000_000;

    for _ in 0..MAX_RETRIES {
        // KVM marks an in-progress pvclock update with an odd `version`.
        // Accept fields only when `version` is even and unchanged across the read.
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst#L87-L90>.
        let version_before = volatile_read_version(pvclock_info);
        if version_before & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }

        compiler_fence(Ordering::Acquire);
        let tsc_to_system_mul = volatile_read_tsc_to_system_mul(pvclock_info);
        let tsc_shift = volatile_read_tsc_shift(pvclock_info);
        compiler_fence(Ordering::Acquire);

        let version_after = volatile_read_version(pvclock_info);
        if version_before == version_after && tsc_to_system_mul != 0 {
            return Some(PvclockTimeSnapshot {
                tsc_to_system_mul,
                tsc_shift,
            });
        }

        core::hint::spin_loop();
    }

    None
}

fn volatile_read_version(pvclock_info: *const PvclockVcpuTimeInfo) -> u32 {
    // SAFETY: `pvclock_info` points to a live KVM pvclock page.
    unsafe { core::ptr::addr_of!((*pvclock_info).version).read_volatile() }
}

fn volatile_read_tsc_to_system_mul(pvclock_info: *const PvclockVcpuTimeInfo) -> u32 {
    // SAFETY: `pvclock_info` points to a live KVM pvclock page.
    unsafe { core::ptr::addr_of!((*pvclock_info).tsc_to_system_mul).read_volatile() }
}

fn volatile_read_tsc_shift(pvclock_info: *const PvclockVcpuTimeInfo) -> i8 {
    // SAFETY: `pvclock_info` points to a live KVM pvclock page.
    unsafe { core::ptr::addr_of!((*pvclock_info).tsc_shift).read_volatile() }
}

fn freq_from_time_info(time_info: &PvclockTimeSnapshot) -> Option<u64> {
    let mut numerator = 1_000_000_000u128.checked_shl(32)?;
    let mut denominator = time_info.tsc_to_system_mul as u128;

    let shift = i32::from(time_info.tsc_shift);
    // Reverse KVM's TSC-to-ns formula. A positive shift scales the multiplier,
    // while a negative shift scales the input cycles before multiplication.
    // Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst#L106-L117>.
    if shift >= 0 {
        denominator = denominator.checked_shl(shift as u32)?;
    } else {
        numerator = numerator.checked_shl((-shift) as u32)?;
    }

    let freq = numerator.checked_div(denominator)?;
    u64::try_from(freq).ok().filter(|freq| *freq != 0)
}
