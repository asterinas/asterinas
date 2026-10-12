// SPDX-License-Identifier: MPL-2.0

//! Tests of deadlines and the public RTC API, including disposable-VM port tests.

use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::{
    cpu::{CpuId, CpuSet, num_cpus},
    early_println,
    prelude::ktest,
    smp::inter_processor_call,
};
use spin::Once;

use super::*;

static RTC: Once<Arc<RtcCmos>> = Once::new();

pub(super) fn rtc() -> &'static Arc<RtcCmos> {
    RTC.call_once(|| {
        let rtc = Arc::new(RtcCmos::try_new().expect("test requires a legacy CMOS RTC"));
        crate::RTC_DRIVER.call_once(|| rtc.clone());
        rtc
    })
}

#[ktest]
fn retry_deadline_handles_timeout_and_counter_wrap() {
    for start in [0, u64::MAX - 2] {
        let mut cycles = start;
        let mut attempts = 0;
        let result: Result<(), RtcError> = retry_until(
            4,
            || {
                attempts += 1;
                Ok(None)
            },
            || {
                let now = cycles;
                cycles = cycles.wrapping_add(1);
                now
            },
        );
        assert_eq!(result, Err(RtcError::Timeout));
        assert_eq!(attempts, 4);
    }
    let mut memory_error_attempts = 0;
    let result: Result<(), RtcError> = retry_until(
        4,
        || {
            memory_error_attempts += 1;
            Err(RtcError::InvalidTime)
        },
        || 0,
    );
    assert_eq!(result, Err(RtcError::InvalidTime));
    assert_eq!(memory_error_attempts, 1);
    let mut attempts = 0;
    assert_eq!(
        retry_until(
            4,
            || {
                attempts += 1;
                Ok(if attempts == 3 { Some(42) } else { None })
            },
            || 0
        ),
        Ok(42)
    );
}

#[ktest]
fn absent_and_read_only_backends_report_errors() {
    let dummy = crate::rtc::RtcDummy;
    assert_eq!(dummy.read_rtc(), Err(RtcError::NoDevice));
    assert_eq!(
        dummy.set_rtc(&crate::rtc::fallback_time()),
        Err(RtcError::NoDevice)
    );
    struct ReadOnly;
    impl Driver for ReadOnly {
        fn try_new() -> Option<Self> {
            Some(Self)
        }
        fn read_rtc(&self) -> Result<SystemTime, RtcError> {
            Ok(crate::rtc::fallback_time())
        }
    }
    assert_eq!(
        ReadOnly.set_rtc(&crate::rtc::fallback_time()),
        Err(RtcError::Unsupported)
    );
}

#[ktest]
fn hardware_rtc_public_read_ticks() {
    rtc();
    let before = crate::read_rtc().unwrap();
    let start = read_tsc();
    let after = loop {
        let after = crate::read_rtc().unwrap();
        if after != before {
            break after;
        }
        assert!(
            read_tsc().wrapping_sub(start) < 3 * tsc_freq(),
            "RTC stopped ticking"
        );
    };
    assert!(after > before);
    early_println!("[rtc-hw] read PASS: before={:?}, after={:?}", before, after);
}

static SMP_READS: AtomicUsize = AtomicUsize::new(0);
static SMP_WRITES: AtomicUsize = AtomicUsize::new(0);
static SMP_ERRORS: AtomicUsize = AtomicUsize::new(0);
static SMP_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static SMP_MAX_ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn sample() -> SystemTime {
    SystemTime {
        year: 2024,
        month: 2,
        day: 29,
        hour: 12,
        minute: 34,
        second: 20,
        nanos: 0,
    }
}

fn concurrent_batch() {
    let cpu = u32::from(CpuId::current_racy()) as u8;
    let active = SMP_ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
    SMP_MAX_ACTIVE.fetch_max(active, Ordering::SeqCst);
    let written = concurrent_time(cpu);
    for _ in 0..8 {
        if crate::set_rtc(written).is_err() {
            SMP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        SMP_WRITES.fetch_add(1, Ordering::Relaxed);
        match crate::read_rtc() {
            Ok(time) if (0..num_cpus() as u8).any(|cpu| time == concurrent_time(cpu)) => {}
            _ => {
                SMP_ERRORS.fetch_add(1, Ordering::Relaxed);
            }
        }
        SMP_READS.fetch_add(1, Ordering::Relaxed);
    }
    SMP_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

fn concurrent_time(cpu: u8) -> SystemTime {
    SystemTime {
        day: cpu + 1,
        hour: cpu,
        minute: 30 + cpu,
        second: 20 + cpu,
        ..sample()
    }
}

#[ktest]
fn hardware_rtc_write_and_concurrent_access() {
    if !ostd::boot::boot_info()
        .kernel_cmdline
        .split_ascii_whitespace()
        .any(|arg| arg == "rtc_test=destructive")
    {
        early_println!("[rtc-hw] write/SMP SKIPPED: requires rtc_test=destructive");
        return;
    }
    let rtc = rtc();
    assert!((1..=8).contains(&num_cpus()));
    let original = crate::read_rtc().unwrap();
    let original_nvram: [u8; 114] = {
        let mut access = rtc.access.lock();
        core::array::from_fn(|index| access.read_register(14 + index as u8))
    };
    let (status_a, status_b) = {
        let mut access = rtc.access.lock();
        (
            access.read_register(Register::StatusA as u8) & 0x7f,
            access.read_register(Register::StatusB as u8),
        )
    };
    for mode in [0, 2, 4, 6] {
        // Change only the data and hour format bits in this disposable guest.
        rtc.access
            .lock()
            .write_register(Register::StatusB as u8, (status_b & !6) | mode);
        for hour in [0, 12, 23] {
            let time = SystemTime { hour, ..sample() };
            crate::set_rtc(time).unwrap();
            assert_eq!(crate::read_rtc(), Ok(time));
        }
        assert_eq!(
            rtc.access.lock().read_register(Register::StatusA as u8) & 0x7f,
            status_a
        );
        assert_eq!(
            rtc.access.lock().read_register(Register::StatusB as u8),
            (status_b & !6) | mode
        );
    }
    rtc.access
        .lock()
        .write_register(Register::StatusB as u8, status_b);
    // A stuck SET bit must produce a bounded error rather than freeze the CPU.
    rtc.access
        .lock()
        .write_register(Register::StatusB as u8, status_b | 0x80);
    let start = read_tsc();
    assert_eq!(crate::read_rtc(), Err(RtcError::Timeout));
    assert_eq!(crate::set_rtc(sample()), Err(RtcError::Timeout));
    assert!(read_tsc().wrapping_sub(start) < tsc_freq());
    rtc.access
        .lock()
        .write_register(Register::StatusB as u8, status_b);
    SMP_READS.store(0, Ordering::Relaxed);
    SMP_WRITES.store(0, Ordering::Relaxed);
    SMP_ERRORS.store(0, Ordering::Relaxed);
    SMP_ACTIVE.store(0, Ordering::SeqCst);
    SMP_MAX_ACTIVE.store(0, Ordering::SeqCst);
    for _ in 0..32 {
        inter_processor_call(&CpuSet::new_full(), concurrent_batch).wait();
    }
    assert_eq!(SMP_ERRORS.load(Ordering::Relaxed), 0);
    assert_eq!(SMP_READS.load(Ordering::Relaxed), 256 * num_cpus());
    assert_eq!(SMP_WRITES.load(Ordering::Relaxed), 256 * num_cpus());
    assert_eq!(SMP_ACTIVE.load(Ordering::SeqCst), 0);
    if num_cpus() > 1 {
        assert!(SMP_MAX_ACTIVE.load(Ordering::SeqCst) >= 2);
    }
    crate::set_rtc(original).unwrap();
    assert_eq!(crate::read_rtc(), Ok(original));
    let restored_nvram: [u8; 114] = {
        let mut access = rtc.access.lock();
        core::array::from_fn(|index| access.read_register(14 + index as u8))
    };
    assert_eq!(restored_nvram, original_nvram);
    early_println!(
        "[rtc-hw] write/SMP PASS: cpus={}, reads={}, writes={}, timeouts=true, nvram_preserved=true, restored=true",
        num_cpus(),
        SMP_READS.load(Ordering::Relaxed),
        SMP_WRITES.load(Ordering::Relaxed)
    );
}
