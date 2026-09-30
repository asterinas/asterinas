// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{Ordering, fence};

use spin::Once;
use x86::msr;

use crate::{
    arch::{cpu::cpuid, io::io_mem::read_once},
    impl_frame_meta_for,
    mm::{self, Frame, FrameAllocOptions, HasPaddr},
};

// KVM feature bits and MSR values.
// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/include/uapi/asm/kvm_para.h>.
const KVM_FEATURE_CLOCKSOURCE2: u32 = 3;
const MSR_KVM_SYSTEM_TIME_NEW: u32 = 0x4b56_4d01;
const KVM_MSR_ENABLE_BIT: u64 = 1;

/// Metadata for the KVM pvclock frame.
///
/// The frame is written by the hypervisor and read by the guest clock code,
/// so it is kept typed to prevent OSTD users from reading or writing it as
/// untyped memory via [`VmReader`] and [`VmWriter`].
///
/// [`VmReader`]: crate::mm::VmReader
/// [`VmWriter`]: crate::mm::VmWriter
struct PvclockPageMeta;
impl_frame_meta_for!(PvclockPageMeta);

/// Guest-visible layout of the KVM pvclock page.
///
/// This is Linux's `struct pvclock_vcpu_time_info`. KVM writes this structure
/// directly into the page we hand it via `MSR_KVM_SYSTEM_TIME_NEW`.
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

/// Shared version-based protocol for reading KVM pvclock data consistently.
///
/// An odd `version` means the host is still writing the pvclock fields.
/// A read is consistent only when the version observed before and after
/// reading the fields is the same even value.
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst#L86-L90>.
///
/// FIXME: All synchronization in this protocol is based on the assumption that
/// we are performing atomic operations under the same memory model as the KVM
/// side. However:
/// 1. The Linux memory model is incompatible with Rust's.
/// 2. We use non-atomic volatile operations to safely communicate with
///    untrusted parties. But these operations cannot establish synchronization.
trait PvclockVersion {
    /// Returns the current pvclock `version` value.
    fn read_version(&self) -> u32;

    /// Begins a consistent read and returns the version to compare against.
    fn read_begin(&self) -> u32 {
        // Masking the odd bit makes `is_version_changed` reject both reads that begin
        // during an update and reads that race with a completed update.
        let version = self.read_version() & !1;

        // Synchronize with the (even) version update so that we can see the data written before
        // the version is written.
        fence(Ordering::Acquire);
        version
    }

    /// Returns whether the pvclock `version` changed since the read was begun.
    fn is_version_changed(&self, version: u32) -> bool {
        // Synchronize with the data update so that we can see the (odd) version update
        // before the data is written (if the data is changed concurrently).
        fence(Ordering::Acquire);
        self.read_version() != version
    }
}

/// A handle to the KVM pvclock page shared with the hypervisor.
struct PvclockPage(Frame<PvclockPageMeta>);

impl PvclockPage {
    /// Sets up the KVM pvclock page and returns a handle to it.
    fn setup() -> Option<&'static Self> {
        static PVCLOCK_PAGE: Once<PvclockPage> = Once::new();

        if !has_kvm_clocksource2() {
            return None;
        }

        if let Some(page) = PVCLOCK_PAGE.get() {
            return Some(page);
        }

        let frame = FrameAllocOptions::new()
            .alloc_frame_with(PvclockPageMeta)
            .ok()?;
        Some(PVCLOCK_PAGE.call_once(|| {
            // SAFETY: `frame` is a live page that will be retained by
            // `PVCLOCK_PAGE`, and this MSR is supported per
            // `has_kvm_clocksource2()` above.
            unsafe {
                msr::wrmsr(
                    MSR_KVM_SYSTEM_TIME_NEW,
                    frame.paddr() as u64 | KVM_MSR_ENABLE_BIT,
                )
            };
            Self(frame)
        }))
    }

    /// Returns a consistent snapshot read from the KVM pvclock page.
    fn read_time_snapshot(&self) -> Option<PvclockTimeSnapshot> {
        const MAX_RETRIES: usize = 1_000_000;

        for _ in 0..MAX_RETRIES {
            let version = self.read_begin();
            let tsc_to_system_mul = self.read_tsc_to_system_mul();
            let tsc_shift = self.read_tsc_shift();
            if !self.is_version_changed(version) && tsc_to_system_mul != 0 {
                return Some(PvclockTimeSnapshot {
                    tsc_to_system_mul,
                    tsc_shift,
                });
            }

            core::hint::spin_loop();
        }

        None
    }

    fn read_tsc_to_system_mul(&self) -> u32 {
        // SAFETY: `self.as_ptr()` points to a live KVM pvclock page.
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).tsc_to_system_mul)) }
    }

    fn read_tsc_shift(&self) -> i8 {
        // SAFETY: `self.as_ptr()` points to a live KVM pvclock page.
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).tsc_shift)) }
    }

    /// Returns a pointer that points to the pvclock page.
    fn as_ptr(&self) -> *const PvclockVcpuTimeInfo {
        mm::paddr_to_vaddr(self.0.paddr()) as *const PvclockVcpuTimeInfo
    }
}

impl PvclockVersion for PvclockPage {
    fn read_version(&self) -> u32 {
        // SAFETY: `self.as_ptr()` points to a live KVM pvclock page.
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).version)) }
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
        // The pvclock ABI encodes the TSC frequency using a scale factor
        // (`tsc_to_system_mul`) and a shift (`tsc_shift`) instead of a direct
        // frequency.
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/pvclock.c#L27>.

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
    let pvclock_page = PvclockPage::setup()?;
    let time_snapshot = pvclock_page.read_time_snapshot()?;
    time_snapshot.tsc_freq_hz()
}

fn has_kvm_clocksource2() -> bool {
    cpuid::query_if_running_under_kvm() && cpuid::query_kvm_feature(KVM_FEATURE_CLOCKSOURCE2)
}
