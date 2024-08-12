// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

//! The Virtual Dynamic Shared Object (VDSO) module enables user space applications to access kernel space routines
//! without the need for context switching. This is particularly useful for frequently invoked operations such as
//! obtaining the current time, which can be more efficiently handled within the user space.
//!
//! This module manages the VDSO mechanism through the `Vdso` struct, which contains a `VdsoData` instance with
//! necessary time-related information, and a Virtual Memory Object (VMO) that encapsulates both the data and the
//! VDSO routines. The VMO is intended to be mapped into the address space of every user space process for efficient access.
//!
//! The module is initialized with `init`, which sets up the `START_SECS_COUNT` and prepares the VDSO instance for
//! use. It also hooks up the VDSO data update routine to the time management subsystem for periodic updates.

use alloc::{boxed::Box, sync::Arc};
use core::{mem::ManuallyDrop, time::Duration};

use aster_rights::Rights;
use aster_time::{read_monotonic_time, Instant};
use aster_util::coeff::Coeff;
use ostd::{
    mm::{Frame, VmIo, PAGE_SIZE},
    sync::SpinLock,
    Pod,
};
use spin::Once;

use crate::{
    fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    syscall::ClockId,
    time::{clocks::MonotonicClock, timer::Timeout, SystemTime, START_TIME},
    vm::vmo::{Vmo, VmoOptions},
};

const CLOCK_TAI: usize = 11;
const VDSO_BASES: usize = CLOCK_TAI + 1;
const DEFAULT_CLOCK_MODE: VdsoClockMode = VdsoClockMode::Tsc;

static START_SECS_COUNT: Once<u64> = Once::new();
static VDSO: Once<Arc<Vdso>> = Once::new();

#[derive(Debug, Copy, Clone)]
enum VdsoClockMode {
    None = 0,
    Tsc = 1,
    Pvclock = 2,
    Hvclock = 3,
    Timens = i32::MAX as isize,
}

/// Instant used in `VdsoData`.
///
/// Each `VdsoInstant` will store a instant information for a specified `ClockId`.
/// The `secs` field will record the seconds of the instant,
/// and the `nanos_info` will store the nanoseconds of the instant
/// (for `CLOCK_REALTIME_COARSE` and `CLOCK_MONOTONIC_COARSE`) or
/// the calculation results of left-shift `nanos` with `lshift`
/// (for other high-resolution `ClockId`s).
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct VdsoInstant {
    secs: u64,
    nanos_info: u64,
}

impl VdsoInstant {
    const fn zero() -> Self {
        Self {
            secs: 0,
            nanos_info: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct ArchVdsoData {}

/// A POD (Plain Old Data) structure maintaining timing information that required for userspace.
///
/// Since currently we directly use the VDSO shared library of Linux,
/// currently it aligns with the Linux VDSO shared library format and contents
/// (Linux v6.2.10)
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct VdsoData {
    seq: u32,

    clock_mode: i32,
    last_cycles: u64,
    mask: u64,
    mult: u32,
    shift: u32,
    basetime: [VdsoInstant; VDSO_BASES],

    tz_minuteswest: i32,
    tz_dsttime: i32,
    hrtimer_res: u32,
    __unused: u32,

    arch_data: ArchVdsoData,
}

const HIGH_RES_CLOCK_IDS: [ClockId; 4] = [
    ClockId::CLOCK_REALTIME,
    ClockId::CLOCK_MONOTONIC,
    ClockId::CLOCK_MONOTONIC_RAW,
    ClockId::CLOCK_BOOTTIME,
];

const COARSE_RES_CLOCK_IDS: [ClockId; 2] = [
    ClockId::CLOCK_REALTIME_COARSE,
    ClockId::CLOCK_MONOTONIC_COARSE,
];

impl VdsoData {
    const fn empty() -> Self {
        VdsoData {
            seq: 0,
            clock_mode: VdsoClockMode::None as i32,
            last_cycles: 0,
            mask: 0,
            mult: 0,
            shift: 0,
            basetime: [VdsoInstant::zero(); VDSO_BASES],
            tz_minuteswest: 0,
            tz_dsttime: 0,
            hrtimer_res: 0,
            __unused: 0,
            arch_data: ArchVdsoData {},
        }
    }

    /// Init VDSO data based on the default clocksource.
    fn init(&mut self) {
        let clocksource = aster_time::default_clocksource();
        let coeff = clocksource.coeff();
        self.set_clock_mode(DEFAULT_CLOCK_MODE);
        self.set_coeff(coeff);

        let (last_instant, last_cycles) = clocksource.last_record();
        self.update_high_res_instant(last_instant, last_cycles);
        self.update_coarse_res_instant(last_instant);
    }

    fn set_clock_mode(&mut self, mode: VdsoClockMode) {
        self.clock_mode = mode as i32;
    }

    fn set_coeff(&mut self, coeff: &Coeff) {
        self.mult = coeff.mult();
        self.shift = coeff.shift();
    }

    fn update_clock_instant(&mut self, clockid: usize, secs: u64, nanos_info: u64) {
        self.basetime[clockid].secs = secs;
        self.basetime[clockid].nanos_info = nanos_info;
    }

    fn update_high_res_instant(&mut self, instant: Instant, instant_cycles: u64) {
        self.last_cycles = instant_cycles;
        for clock_id in HIGH_RES_CLOCK_IDS {
            let secs = if clock_id == ClockId::CLOCK_REALTIME {
                instant.secs() + START_SECS_COUNT.get().unwrap()
            } else {
                instant.secs()
            };

            self.update_clock_instant(
                clock_id as usize,
                secs,
                (instant.nanos() as u64) << self.shift as u64,
            );
        }
    }

    fn update_coarse_res_instant(&mut self, instant: Instant) {
        for clock_id in COARSE_RES_CLOCK_IDS {
            let secs = if clock_id == ClockId::CLOCK_REALTIME_COARSE {
                instant.secs() + START_SECS_COUNT.get().unwrap()
            } else {
                instant.secs()
            };
            self.update_clock_instant(clock_id as usize, secs, instant.nanos() as u64);
        }
    }
}

/// Vdso (virtual dynamic shared object) is used to export some safe kernel space routines to user space applications
/// so that applications can call these kernel space routines in-process, without context switching.
///
/// Vdso maintains a `VdsoData` instance that contains data information required for VDSO mechanism,
/// and a `Vmo` that contains all VDSO-related information, including the VDSO data and the VDSO calling interfaces.
/// This `Vmo` must be mapped to every userspace process.
struct Vdso {
    /// A `VdsoData` instance.
    data: SpinLock<VdsoData>,
    /// The VMO of the entire VDSO, including the library text and the VDSO data.
    vmo: Arc<Vmo>,
    /// The `Frame` that contains the VDSO data. This frame is contained in and
    /// will not be removed from the VDSO VMO.
    data_frame: Frame,
}

/// A `SpinLock` for the `seq` field in `VdsoData`.
static SEQ_LOCK: SpinLock<()> = SpinLock::new(());

/// The size of the VDSO VMO.
pub const VDSO_VMO_SIZE: usize = 5 * PAGE_SIZE;

impl Vdso {
    /// Construct a new `Vdso`, including an initialized `VdsoData` and a VMO of the VDSO.
    fn new() -> Self {
        let mut vdso_data = VdsoData::empty();
        vdso_data.init();

        let (vdso_vmo, data_frame) = {
            let vmo_options = VmoOptions::<Rights>::new(VDSO_VMO_SIZE);
            let vdso_vmo = vmo_options.alloc().unwrap();
            // Write VDSO data to VDSO VMO.
            vdso_vmo.write_bytes(0x80, vdso_data.as_bytes()).unwrap();

            let vdso_lib_vmo = {
                let vdso_path = FsPath::new(AT_FDCWD, "/lib/x86_64-linux-gnu/vdso64.so").unwrap();
                let fs_resolver = FsResolver::new();
                let vdso_lib = fs_resolver.lookup(&vdso_path).unwrap();
                vdso_lib.inode().page_cache().unwrap()
            };
            let mut vdso_text = Box::new([0u8; PAGE_SIZE]);
            vdso_lib_vmo.read_bytes(0, &mut *vdso_text).unwrap();
            // Write VDSO library to VDSO VMO.
            vdso_vmo.write_bytes(0x4000, &*vdso_text).unwrap();

            let data_frame = vdso_vmo.commit_page(0).unwrap();
            (vdso_vmo, data_frame)
        };
        Self {
            data: SpinLock::new(vdso_data),
            vmo: Arc::new(vdso_vmo),
            data_frame,
        }
    }

    fn update_high_res_instant(&self, instant: Instant, instant_cycles: u64) {
        let seq_lock = SEQ_LOCK.lock();
        self.data
            .lock()
            .update_high_res_instant(instant, instant_cycles);

        // Update begins.
        self.data_frame.write_val(0x80, &1).unwrap();
        self.data_frame.write_val(0x88, &instant_cycles).unwrap();
        for clock_id in HIGH_RES_CLOCK_IDS {
            self.update_data_frame_instant(clock_id);
        }

        // Update finishes.
        self.data_frame.write_val(0x80, &0).unwrap();
    }

    fn update_coarse_res_instant(&self, instant: Instant) {
        let seq_lock = SEQ_LOCK.lock();
        self.data.lock().update_coarse_res_instant(instant);

        // Update begins.
        self.data_frame.write_val(0x80, &1).unwrap();
        for clock_id in COARSE_RES_CLOCK_IDS {
            self.update_data_frame_instant(clock_id);
        }

        // Update finishes.
        self.data_frame.write_val(0x80, &0).unwrap();
    }

    /// Update the requisite fields of the VDSO data in the `data_frame`.
    fn update_data_frame_instant(&self, clockid: ClockId) {
        let clock_index = clockid as usize;
        let secs_offset = 0xA0 + clock_index * 0x10;
        let nanos_info_offset = 0xA8 + clock_index * 0x10;
        let data = self.data.lock();
        self.data_frame
            .write_val(secs_offset, &data.basetime[clock_index].secs)
            .unwrap();
        self.data_frame
            .write_val(nanos_info_offset, &data.basetime[clock_index].nanos_info)
            .unwrap();
    }
}

/// Update the `VdsoInstant` for clock IDs with high resolution in Vdso.
fn update_vdso_high_res_instant(instant: Instant, instant_cycles: u64) {
    VDSO.get()
        .unwrap()
        .update_high_res_instant(instant, instant_cycles);
}

/// Update the `VdsoInstant` for clock IDs with coarse resolution in Vdso.
fn update_vdso_coarse_res_instant() {
    let instant = Instant::from(read_monotonic_time());
    VDSO.get().unwrap().update_coarse_res_instant(instant);
}

/// Init `START_SECS_COUNT`, which is used to record the seconds passed since 1970-01-01 00:00:00.
fn init_start_secs_count() {
    let time_duration = START_TIME
        .get()
        .unwrap()
        .duration_since(&SystemTime::UNIX_EPOCH)
        .unwrap();
    START_SECS_COUNT.call_once(|| time_duration.as_secs());
}

fn init_vdso() {
    let vdso = Vdso::new();
    VDSO.call_once(|| Arc::new(vdso));
}

/// Init this module.
pub(super) fn init() {
    init_start_secs_count();
    init_vdso();
    aster_time::VDSO_DATA_HIGH_RES_UPDATE_FN.call_once(|| Arc::new(update_vdso_high_res_instant));

    // Coarse resolution clock IDs directly read the instant stored in VDSO data without
    // using coefficients for calculation, thus the related instant requires more frequent updating.
    let coarse_instant_timer = ManuallyDrop::new(
        MonotonicClock::timer_manager().create_timer(update_vdso_coarse_res_instant),
    );
    coarse_instant_timer.set_interval(Duration::from_millis(100));
    coarse_instant_timer.set_timeout(Timeout::After(Duration::from_millis(100)));
}

/// Returns the VDSO VMO.
pub(crate) fn vdso_vmo() -> Option<Arc<Vmo>> {
    // We allow that VDSO does not exist
    VDSO.get().map(|vdso| vdso.vmo.clone())
}
