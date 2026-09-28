// SPDX-License-Identifier: MPL-2.0

//! Port-I/O regression tests. Destructive tests require an explicit boot argument.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use ostd::{
    arch::{read_tsc, tsc_freq},
    cpu::{CpuId, CpuSet, num_cpus},
    early_println,
    prelude::ktest,
    smp::inter_processor_call,
    task::atomic_mode::might_sleep,
};
use spin::Once;

use super::{Driver, RtcCmos, nvram::Access};
use crate::{NvramError, SystemTime};

static RTC: Once<Arc<RtcCmos>> = Once::new();
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static MAX_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CPU_MASK: AtomicU64 = AtomicU64::new(0);
static ERRORS: AtomicUsize = AtomicUsize::new(0);
static WRITES: AtomicUsize = AtomicUsize::new(0);
static READS: AtomicUsize = AtomicUsize::new(0);
static RTC_READS: AtomicUsize = AtomicUsize::new(0);

fn rtc() -> &'static Arc<RtcCmos> {
    RTC.call_once(|| {
        let rtc = Arc::new(RtcCmos::try_new().expect(
            "CMOS regression requires the legacy RTC and exclusive ownership of its ports",
        ));
        crate::RTC_DRIVER.call_once(|| rtc.clone());
        rtc
    })
}

fn valid_time(time: &SystemTime) -> bool {
    (1970..=9999).contains(&time.year)
        && (1..=12).contains(&time.month)
        && (1..=31).contains(&time.day)
        && time.hour < 24
        && time.minute < 60
        && time.second < 60
}

fn raw_snapshot(rtc: &RtcCmos) -> [u8; super::nvram::SIZE] {
    let mut access = rtc.access.lock();
    core::array::from_fn(|index| access.read_byte(index as u8))
}

struct RestoreNvram {
    rtc: &'static RtcCmos,
    original: [u8; super::nvram::SIZE],
}

impl RestoreNvram {
    fn new(rtc: &'static RtcCmos) -> Self {
        Self {
            rtc,
            original: raw_snapshot(rtc),
        }
    }

    fn restore(&self) {
        let mut access = self.rtc.access.lock();
        for (index, byte) in self.original.iter().enumerate() {
            access.write_byte(index as u8, *byte);
        }
    }
}

impl Drop for RestoreNvram {
    fn drop(&mut self) {
        self.restore();
    }
}

#[ktest]
fn rtc_cmos_read_only_regression() {
    let rtc = rtc();
    might_sleep();
    let original = raw_snapshot(rtc);
    let before = rtc.read_rtc();
    assert!(valid_time(&before));
    assert_eq!(crate::nvram_size(), Ok(super::nvram::SIZE));
    for _ in 0..128 {
        let mut bytes = [0; super::nvram::SIZE];
        match crate::nvram_read(0, &mut bytes) {
            Ok(length) => {
                assert_eq!(length, bytes.len());
                assert_eq!(bytes, original);
            }
            Err(NvramError::InvalidChecksum) => {}
            other => panic!("unexpected CMOS read result: {other:?}"),
        }
        assert!(valid_time(&rtc.read_rtc()));
    }
    let start = read_tsc();
    let after = loop {
        let after = rtc.read_rtc();
        if after != before {
            break after;
        }
        assert!(
            read_tsc().wrapping_sub(start) < 3 * tsc_freq(),
            "RTC stopped ticking"
        );
    };
    assert!(valid_time(&after));
    assert!(after > before);
    assert_eq!(raw_snapshot(rtc), original);
    might_sleep();
    early_println!(
        "[nvram-hw] read-only PASS: before={:?}, after={:?}",
        before,
        after
    );
}

// Each IPI performs a bounded batch. It never waits for another worker, allocates,
// or accesses user memory. All port accesses use the production shared lock.
fn concurrent_port_batch() {
    let cpu = u32::from(CpuId::current_racy()) as usize;
    CPU_MASK.fetch_or(1_u64 << cpu, Ordering::Relaxed);
    let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
    MAX_ACTIVE.fetch_max(active, Ordering::SeqCst);
    let rtc = RTC.get().unwrap();
    for _ in 0..8 {
        if cpu.is_multiple_of(2) {
            let payload = [0x80 + cpu as u8; 30];
            if crate::nvram_write(2, &payload) != Ok(payload.len()) {
                ERRORS.fetch_add(1, Ordering::Relaxed);
            }
            WRITES.fetch_add(1, Ordering::Relaxed);
        } else {
            if !valid_time(&rtc.read_rtc()) {
                ERRORS.fetch_add(1, Ordering::Relaxed);
            }
            RTC_READS.fetch_add(1, Ordering::Relaxed);
        }
        let mut output = [0; 30];
        if crate::nvram_read(2, &mut output) != Ok(output.len())
            || !(0x80..0x80 + num_cpus() as u8).contains(&output[0])
            || output.iter().any(|byte| *byte != output[0])
        {
            ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        READS.fetch_add(1, Ordering::Relaxed);
    }
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

#[ktest]
fn nvram_cmos_write_and_smp_regression() {
    // These writes change firmware settings, including the century byte during
    // initialization. Run only in a disposable VM or an authorized test machine.
    // Restoration cannot protect against power loss or a non-unwinding panic.
    if !ostd::boot::boot_info()
        .kernel_cmdline
        .split_ascii_whitespace()
        .any(|arg| arg == "nvram_test=destructive")
    {
        early_println!("[nvram-hw] write/SMP SKIPPED: requires nvram_test=destructive");
        return;
    }
    assert!(
        (2..=64).contains(&num_cpus()),
        "SMP regression requires 2..=64 CPUs"
    );
    let rtc = rtc();
    let restore = RestoreNvram::new(rtc);
    let before = rtc.read_rtc();
    assert!(valid_time(&before));
    let status_b = rtc.access.lock().read_status_b().bits();

    crate::nvram_initialize().unwrap();
    // CMOS 0x32 and 0x37 are RTC century registers/aliases on PC platforms.
    // QEMU recalculates them from its clock on reads after initialization.
    // These locations are not ordinary SRAM; the algorithm tests separately
    // prove that initialization writes zero to every byte.
    let initialized = raw_snapshot(rtc);
    let century = rtc.access.lock().century_register.map(|r| r.get());
    for (offset, byte) in initialized.iter().enumerate() {
        let register = super::nvram::FIRST_BYTE + offset as u8;
        if register != 0x32 && register != 0x37 && Some(register) != century {
            assert_eq!(*byte, 0, "CMOS register {register:#x} was not cleared");
        }
    }
    restore.restore();
    crate::nvram_set_checksum().unwrap();
    assert_eq!(crate::nvram_write(2, &[0x80; 30]), Ok(30));
    let mut payload = [0; 30];
    assert_eq!(crate::nvram_read(2, &mut payload), Ok(30));
    assert_eq!(payload, [0x80; 30]);
    assert_eq!(crate::nvram_read(usize::MAX, &mut payload), Ok(0));
    assert_eq!(crate::nvram_write(usize::MAX, &payload), Ok(0));
    {
        let mut access = rtc.access.lock();
        access.write_byte(2, 0x81);
    }
    assert_eq!(
        crate::nvram_read(0, &mut []),
        Err(NvramError::InvalidChecksum)
    );
    assert_eq!(
        crate::nvram_write(2, &payload),
        Err(NvramError::InvalidChecksum)
    );
    crate::nvram_set_checksum().unwrap();
    assert_eq!(crate::nvram_write(2, &[0x80; 30]), Ok(30));

    for counter in [&ACTIVE, &MAX_ACTIVE, &ERRORS, &WRITES, &READS, &RTC_READS] {
        counter.store(0, Ordering::SeqCst);
    }
    CPU_MASK.store(0, Ordering::SeqCst);
    for _ in 0..128 {
        inter_processor_call(&CpuSet::new_full(), concurrent_port_batch).wait();
    }
    assert_eq!(ERRORS.load(Ordering::SeqCst), 0);
    assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    assert_eq!(
        CPU_MASK.load(Ordering::SeqCst).count_ones() as usize,
        num_cpus()
    );
    assert_eq!(READS.load(Ordering::SeqCst), 1024 * num_cpus());
    assert!(
        MAX_ACTIVE.load(Ordering::SeqCst) >= 2,
        "no overlapping CPU workers observed"
    );
    assert!(WRITES.load(Ordering::SeqCst) > 0);
    assert!(RTC_READS.load(Ordering::SeqCst) > 0);
    might_sleep();
    restore.restore();
    assert_eq!(raw_snapshot(rtc), restore.original);
    assert_eq!(rtc.access.lock().read_status_b().bits(), status_b);
    let after = rtc.read_rtc();
    assert!(valid_time(&after));
    assert!(after >= before);
    early_println!(
        "[nvram-hw] write/SMP PASS: cpus={}, mask={:#x}, max_active={}, writes={}, reads={}, rtc_reads={}, restored=true",
        num_cpus(),
        CPU_MASK.load(Ordering::SeqCst),
        MAX_ACTIVE.load(Ordering::SeqCst),
        WRITES.load(Ordering::SeqCst),
        READS.load(Ordering::SeqCst),
        RTC_READS.load(Ordering::SeqCst)
    );
}
