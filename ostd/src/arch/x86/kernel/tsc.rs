// SPDX-License-Identifier: MPL-2.0

use core::{
    ptr::read_volatile,
    sync::atomic::{AtomicBool, AtomicU64, Ordering, compiler_fence},
};

use x86::msr::wrmsr;

use crate::{
    arch::{
        cpu::cpuid::cpuid,
        timer::pit::{self, OperatingMode},
        trap::TrapFrame,
    },
    info,
    irq::IrqLine,
    mm::kspace::kernel_loaded_offset,
    timer::TIMER_FREQ,
};

/// The frequency in Hz of the Time Stamp Counter (TSC).
pub(in crate::arch) static TSC_FREQ: AtomicU64 = AtomicU64::new(0);

pub fn init_tsc_freq() {
    use crate::arch::cpu::cpuid::{
        query_tsc_freq as determine_tsc_freq_via_cpuid,
        query_tsc_freq_via_hypervisor as determine_tsc_freq_via_hypervisor,
    };

    // Ask the hypervisor before falling back to the PIT: a VMM that does not enumerate the
    // nominal core crystal clock may not emulate a PIT either, in which case the PIT-based
    // calibration would wait forever for an interrupt that never arrives.
    let tsc_freq = determine_tsc_freq_via_cpuid()
        .or_else(determine_tsc_freq_via_hypervisor)
        .or_else(determine_tsc_freq_via_kvm_clock)
        .unwrap_or_else(determine_tsc_freq_via_pit);
    TSC_FREQ.store(tsc_freq, Ordering::Relaxed);
    info!("TSC frequency: {:?} Hz", tsc_freq);
}

/// The system time structure of the KVM paravirtualized clock ("kvm-clock").
///
/// Reference: <https://www.kernel.org/doc/html/latest/virt/kvm/x86/msr.html>
#[repr(C, align(64))]
struct PvclockVcpuTimeInfo {
    version: u32,
    pad0: u32,
    tsc_timestamp: u64,
    system_time: u64,
    tsc_to_system_mul: u32,
    tsc_shift: i8,
    flags: u8,
    pad1: [u8; 2],
}

/// The memory that the hypervisor fills with kvm-clock timing information.
///
/// It must live in the kernel image (rather than on the stack or the heap) because its physical
/// address is handed to the hypervisor.
static mut PVCLOCK_TIME_INFO: PvclockVcpuTimeInfo = PvclockVcpuTimeInfo {
    version: 0,
    pad0: 0,
    tsc_timestamp: 0,
    system_time: 0,
    tsc_to_system_mul: 0,
    tsc_shift: 0,
    flags: 0,
    pad1: [0; 2],
};

/// Determines the TSC frequency with the help of the KVM paravirtualized clock ("kvm-clock").
///
/// Some VMMs enumerate neither the nominal core crystal clock nor the processor base frequency in
/// the results of the CPUID instruction, and emulate no Programmable Interval Timer (PIT) either.
/// Cloud Hypervisor and Firecracker are two such VMMs. On them the paravirtualized clock is the
/// only way to learn the TSC frequency, and without it the PIT-based calibration would wait
/// forever for an interrupt that never arrives.
///
/// This method will return `None` if the hypervisor is not KVM-compatible or does not provide the
/// paravirtualized clock.
pub fn determine_tsc_freq_via_kvm_clock() -> Option<u64> {
    /// The "KVMKVMKVM\0\0\0" signature reported in the hypervisor CPUID leaf.
    const KVM_SIGNATURE: [u32; 3] = [0x4b4d_564b, 0x564b_4d56, 0x0000_004d];
    /// The feature bit for the paravirtualized clock, which uses [`MSR_KVM_SYSTEM_TIME`].
    const KVM_FEATURE_CLOCKSOURCE: u32 = 1 << 0;
    /// The feature bit for the paravirtualized clock, which uses [`MSR_KVM_SYSTEM_TIME_NEW`].
    const KVM_FEATURE_CLOCKSOURCE2: u32 = 1 << 3;
    const MSR_KVM_SYSTEM_TIME: u32 = 0x12;
    const MSR_KVM_SYSTEM_TIME_NEW: u32 = 0x4b56_4d01;
    /// The number of attempts to read a consistent snapshot before giving up.
    const MAX_READ_ATTEMPTS: usize = 100;

    let signature = cpuid(0x4000_0000, 0)?;
    if [signature.ebx, signature.ecx, signature.edx] != KVM_SIGNATURE {
        return None;
    }

    let features = cpuid(0x4000_0001, 0)?.eax;
    let msr = if features & KVM_FEATURE_CLOCKSOURCE2 != 0 {
        MSR_KVM_SYSTEM_TIME_NEW
    } else if features & KVM_FEATURE_CLOCKSOURCE != 0 {
        MSR_KVM_SYSTEM_TIME
    } else {
        return None;
    };

    let time_info = &raw mut PVCLOCK_TIME_INFO;
    // The kernel image is linearly mapped, so the physical address of a static can be computed by
    // subtracting the offset that the kernel is loaded at.
    let paddr = time_info as usize - kernel_loaded_offset();

    // SAFETY: `PVCLOCK_TIME_INFO` is a valid, suitably aligned structure that lives for the
    // lifetime of the kernel, so it is safe to let the hypervisor write timing information to it.
    // The lowest bit enables the clock.
    unsafe { wrmsr(msr, paddr as u64 | 1) };

    // The hypervisor updates the structure concurrently, using `version` as a seqlock: it is odd
    // while an update is in progress, and is bumped after each update.
    let mut snapshot = None;
    for _ in 0..MAX_READ_ATTEMPTS {
        // SAFETY: The structure is valid and the hypervisor only ever writes to it.
        let (before, mul, shift, after) = unsafe {
            let before = read_volatile(&raw const (*time_info).version);
            compiler_fence(Ordering::Acquire);
            let mul = read_volatile(&raw const (*time_info).tsc_to_system_mul);
            let shift = read_volatile(&raw const (*time_info).tsc_shift);
            compiler_fence(Ordering::Acquire);
            let after = read_volatile(&raw const (*time_info).version);
            (before, mul, shift, after)
        };

        if before == after && before % 2 == 0 {
            snapshot = Some((mul, shift));
            break;
        }
    }

    // SAFETY: Writing zero disables the clock, so that the hypervisor stops writing to the
    // structure once we no longer need it.
    unsafe { wrmsr(msr, 0) };

    let (tsc_to_system_mul, tsc_shift) = snapshot?;
    if tsc_to_system_mul == 0 {
        return None;
    }

    // The hypervisor describes the TSC frequency by the factors that convert a TSC value to
    // nanoseconds: `ns = tsc * tsc_to_system_mul * 2^tsc_shift / 2^32`. Inverting that gives the
    // frequency in kHz, so the shift is applied in the opposite direction.
    let mut tsc_freq_khz = (1_000_000_u64 << 32) / tsc_to_system_mul as u64;
    if tsc_shift < 0 {
        tsc_freq_khz <<= (-tsc_shift) as u32;
    } else {
        tsc_freq_khz >>= tsc_shift as u32;
    }

    Some(tsc_freq_khz * 1_000)
}

/// Determines the TSC frequency with the help of the Programmable Interval Timer (PIT).
///
/// When the TSC frequency is not enumerated in the results of the CPUID instruction, it can
/// leverage the PIT to calculate the TSC frequency.
pub fn determine_tsc_freq_via_pit() -> u64 {
    // Allocate IRQ
    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(pit_callback);

    // Enable PIT
    pit::init(OperatingMode::RateGenerator);
    let irq = pit::enable_interrupt(irq);

    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    static FREQUENCY: AtomicU64 = AtomicU64::new(0);

    // Wait until `FREQUENCY` is ready
    loop {
        crate::arch::irq::enable_local_and_halt();

        // Disable local IRQs so they won't come after checking `IS_FINISH`
        // but before halting the CPU.
        crate::arch::irq::disable_local();

        if IS_FINISH.load(Ordering::Acquire) {
            break;
        }
    }

    // Disable PIT
    drop(irq);

    return FREQUENCY.load(Ordering::Acquire);

    fn pit_callback(_trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static TSC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        // Set a certain times of callbacks to calculate the frequency
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        let tsc_current_count = crate::arch::read_tsc();

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || IS_FINISH.load(Ordering::Acquire) {
            if IN_TIME.load(Ordering::Relaxed) == 0 {
                TSC_FIRST_COUNT.store(tsc_current_count, Ordering::Relaxed);
            }
            IN_TIME.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let tsc_first_count = TSC_FIRST_COUNT.load(Ordering::Relaxed);
        let freq = (tsc_current_count - tsc_first_count) * (TIMER_FREQ / CALLBACK_TIMES);
        FREQUENCY.store(freq, Ordering::Release);
        IS_FINISH.store(true, Ordering::Release);
    }
}
