// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{Ordering, compiler_fence};

use spin::Once;
use x86::msr;

use crate::{
    arch::cpu::cpuid,
    mm::{self, Frame, FrameAllocOptions, HasPaddr},
};

// KVM CPUID leaves, signature, feature bits, and MSR values.
// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/include/uapi/asm/kvm_para.h>.
const KVM_CPUID_SIGNATURE: u32 = 0x4000_0000;
const KVM_CPUID_FEATURES: u32 = 0x4000_0001;
const KVM_SIGNATURE: &[u8; 12] = b"KVMKVMKVM\0\0\0";
const KVM_FEATURE_CLOCKSOURCE2: u32 = 3;
const MSR_KVM_SYSTEM_TIME_NEW: u32 = 0x4b56_4d01;
const KVM_MSR_ENABLE_BIT: u64 = 1;

static PVCLOCK_FRAME: Once<Option<Frame<()>>> = Once::new();

/// Guest-visible layout of the KVM pvclock page, mirroring the ABI's
/// `struct pvclock_vcpu_time_info`. KVM writes this structure directly
/// into the page we hand it via `MSR_KVM_SYSTEM_TIME_NEW`.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/include/asm/pvclock-abi.h#L26>.
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

/// A handle to the KVM pvclock page shared with the hypervisor.
struct PvclockPage {
    info: *const PvclockVcpuTimeInfo,
}

impl PvclockPage {
    /// Sets up the KVM pvclock page and returns a handle to it.
    fn setup() -> Option<Self> {
        let frame = PVCLOCK_FRAME.call_once(|| FrameAllocOptions::new().alloc_frame().ok());
        let frame = frame.as_ref()?;
        let paddr = frame.paddr();

        // SAFETY: `paddr` is the physical address of a live, zeroed page retained by
        // `PVCLOCK_FRAME`. KVM owns updates to the `PvclockVcpuTimeInfo` fields after
        // this MSR is enabled.
        unsafe {
            msr::wrmsr(MSR_KVM_SYSTEM_TIME_NEW, paddr as u64 | KVM_MSR_ENABLE_BIT);
        }
        Some(Self {
            info: mm::paddr_to_vaddr(paddr) as *const PvclockVcpuTimeInfo,
        })
    }

    /// Returns a consistent snapshot read from the KVM pvclock page.
    fn read_time_snapshot(&self) -> Option<PvclockTimeSnapshot> {
        const MAX_RETRIES: usize = 1_000_000;

        for _ in 0..MAX_RETRIES {
            // KVM marks an in-progress pvclock update with an odd `version`.
            // Accept fields only when `version` is even and unchanged across the read.
            // Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst#L86-L90>.
            let version_before = self.read_version();
            if version_before & 1 != 0 {
                core::hint::spin_loop();
                continue;
            }

            compiler_fence(Ordering::Acquire);
            let tsc_to_system_mul = self.read_tsc_to_system_mul();
            let tsc_shift = self.read_tsc_shift();
            compiler_fence(Ordering::Acquire);

            let version_after = self.read_version();
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

    fn read_version(&self) -> u32 {
        // SAFETY: `self.info` points to a live KVM pvclock page.
        unsafe { core::ptr::addr_of!((*self.info).version).read_volatile() }
    }

    fn read_tsc_to_system_mul(&self) -> u32 {
        // SAFETY: `self.info` points to a live KVM pvclock page.
        unsafe { core::ptr::addr_of!((*self.info).tsc_to_system_mul).read_volatile() }
    }

    fn read_tsc_shift(&self) -> i8 {
        // SAFETY: `self.info` points to a live KVM pvclock page.
        unsafe { core::ptr::addr_of!((*self.info).tsc_shift).read_volatile() }
    }
}

/// A consistent snapshot read from the KVM pvclock page.
struct PvclockTimeSnapshot {
    tsc_to_system_mul: u32,
    tsc_shift: i8,
}

impl PvclockTimeSnapshot {
    /// Returns the TSC frequency in Hz derived from this snapshot.
    fn tsc_freq_hz(&self) -> Option<u64> {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/pvclock.c#L27>.

        // 10^6 << 32 / tsc_to_system_mul gives the TSC frequency in kHz.
        let base_khz = (1_000_000u64 << 32).checked_div(u64::from(self.tsc_to_system_mul))?;

        let tsc_khz = if self.tsc_shift < 0 {
            base_khz.checked_shl((self.tsc_shift as i32).unsigned_abs())?
        } else {
            base_khz.checked_shr(self.tsc_shift as u32)?
        };

        let freq = tsc_khz.checked_mul(1000)?;
        (freq != 0).then_some(freq)
    }
}

/// Determines the TSC frequency from KVM's paravirtual clock.
pub(super) fn determine_tsc_freq() -> Option<u64> {
    if !has_kvm_clocksource2() {
        return None;
    }

    let pvclock_page = PvclockPage::setup()?;
    let time_snapshot = pvclock_page.read_time_snapshot()?;
    time_snapshot.tsc_freq_hz()
}

fn has_kvm_clocksource2() -> bool {
    let Some(signature_leaf) = cpuid::cpuid(KVM_CPUID_SIGNATURE, 0) else {
        return false;
    };

    let mut signature = [0u8; 12];
    signature[0..4].copy_from_slice(&signature_leaf.ebx.to_ne_bytes());
    signature[4..8].copy_from_slice(&signature_leaf.ecx.to_ne_bytes());
    signature[8..12].copy_from_slice(&signature_leaf.edx.to_ne_bytes());
    if &signature != KVM_SIGNATURE {
        return false;
    }

    let Some(features) = cpuid::cpuid(KVM_CPUID_FEATURES, 0) else {
        return false;
    };
    features.eax & (1 << KVM_FEATURE_CLOCKSOURCE2) != 0
}
