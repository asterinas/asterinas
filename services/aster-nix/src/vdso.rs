// SPDX-License-Identifier: MPL-2.0

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

use alloc::boxed::Box;
use alloc::sync::Arc;
use aster_frame::{config::PAGE_SIZE, sync::Mutex, vm::VmIo};
use aster_rights::Rights;
use aster_time::Instant;
use aster_util::coeff::Coeff;
use pod::Pod;
use spin::Once;

use crate::{
    fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    time::{ClockID, SystemTime, ALL_SUPPORTED_CLOCK_IDS},
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
/// The `VdsoInstant` records the second of an instant,
/// and the calculation results of multiplying `nanos` with `mult` in the corresponding `VdsoData`.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct VdsoInstant {
    secs: u64,
    nanos_lshift: u64,
}

impl VdsoInstant {
    const fn zero() -> Self {
        Self {
            secs: 0,
            nanos_lshift: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct ArchVdsoData {}

/// A POD (Plain Old Data) structure maintaining timing information that required for userspace.
///
/// Since currently we directly use the vdso shared library of Linux,
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

    /// Init vdso data based on the default clocksource.
    fn init(&mut self) {
        let clocksource = aster_time::default_clocksource();
        let coeff = clocksource.coeff();
        self.set_clock_mode(DEFAULT_CLOCK_MODE);
        self.set_coeff(coeff);
        self.update_instant(clocksource.last_instant(), clocksource.last_cycles());
    }

    fn set_clock_mode(&mut self, mode: VdsoClockMode) {
        self.clock_mode = mode as i32;
    }

    fn set_coeff(&mut self, coeff: &Coeff) {
        self.mult = coeff.mult();
        self.shift = coeff.shift();
    }

    fn update_clock_instant(&mut self, clockid: usize, secs: u64, nanos_lshift: u64) {
        self.basetime[clockid].secs = secs;
        self.basetime[clockid].nanos_lshift = nanos_lshift;
    }

    fn update_instant(&mut self, instant: Instant, instant_cycles: u64) {
        self.last_cycles = instant_cycles;
        const REALTIME_IDS: [ClockID; 2] =
            [ClockID::CLOCK_REALTIME, ClockID::CLOCK_REALTIME_COARSE];
        for clock_id in ALL_SUPPORTED_CLOCK_IDS {
            let secs = if REALTIME_IDS.contains(&clock_id) {
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
}

/// Vdso (virtual dynamic shared object) is used to export some safe kernel space routines to user space applications
/// so that applications can call these kernel space routines in-process, without context switching.
///
/// Vdso maintains a `VdsoData` instance that contains data information required for vdso mechanism,
/// and a `Vmo` that contains all vdso-related information, including the vdso data and the vdso calling interfaces.
/// This `Vmo` must be mapped to every userspace process.
struct Vdso {
    /// A VdsoData instance.
    data: Mutex<VdsoData>,
    /// the vmo of the entire vdso, including the library text and the vdso data.
    vmo: Arc<Vmo>,
}

impl Vdso {
    /// Construct a new Vdso, including an initialized `VdsoData` and a vmo of the vdso.
    fn new() -> Self {
        let mut vdso_data = VdsoData::empty();
        vdso_data.init();

        let vdso_vmo = {
            let vmo_options = VmoOptions::<Rights>::new(5 * PAGE_SIZE);
            let vdso_vmo = vmo_options.alloc().unwrap();
            // Write vdso data to vdso vmo.
            vdso_vmo.write_bytes(0x80, vdso_data.as_bytes()).unwrap();

            let vdso_lib_vmo = {
                let vdso_path = FsPath::new(AT_FDCWD, "/lib/x86_64-linux-gnu/vdso64.so").unwrap();
                let fs_resolver = FsResolver::new();
                let vdso_lib = fs_resolver.lookup(&vdso_path).unwrap();
                vdso_lib.inode().page_cache().unwrap()
            };
            let mut vdso_text = Box::new([0u8; PAGE_SIZE]);
            vdso_lib_vmo.read_bytes(0, &mut *vdso_text).unwrap();
            // Write vdso library to vdso vmo.
            vdso_vmo.write_bytes(0x4000, &*vdso_text).unwrap();

            vdso_vmo
        };
        Self {
            data: Mutex::new(vdso_data),
            vmo: Arc::new(vdso_vmo),
        }
    }

    /// Return the vdso vmo.
    fn vmo(&self) -> Arc<Vmo> {
        self.vmo.clone()
    }

    fn update_instant(&self, instant: Instant, instant_cycles: u64) {
        self.data.lock().update_instant(instant, instant_cycles);

        // Update begins.
        self.vmo.write_val(0x80, &1).unwrap();
        self.vmo.write_val(0x88, &instant_cycles).unwrap();
        for clock_id in ALL_SUPPORTED_CLOCK_IDS {
            self.update_vmo_instant(clock_id);
        }
        // Update finishes.
        self.vmo.write_val(0x80, &0).unwrap();
    }

    /// Update the requisite fields of the vdso data in the vmo.
    fn update_vmo_instant(&self, clockid: ClockID) {
        let clock_index = clockid as usize;
        let secs_offset = 0xA0 + clock_index * 0x10;
        let nanos_lshift_offset = 0xA8 + clock_index * 0x10;
        let data = self.data.lock();
        self.vmo
            .write_val(secs_offset, &data.basetime[clock_index].secs)
            .unwrap();
        self.vmo
            .write_val(
                nanos_lshift_offset,
                &data.basetime[clock_index].nanos_lshift,
            )
            .unwrap();
    }
}

/// Update the `VdsoInstant` in Vdso.
fn update_vdso_instant(instant: Instant, instant_cycles: u64) {
    VDSO.get().unwrap().update_instant(instant, instant_cycles);
}

/// Init `START_SECS_COUNT`, which is used to record the seconds passed since 1970-01-01 00:00:00.
fn init_start_secs_count() {
    let now = SystemTime::now();
    let time_duration = now.duration_since(&SystemTime::UNIX_EPOCH).unwrap();
    START_SECS_COUNT.call_once(|| time_duration.as_secs());
}

fn init_vdso() {
    let vdso = Vdso::new();
    VDSO.call_once(|| Arc::new(vdso));
}

/// Init vdso module.
pub(super) fn init() {
    init_start_secs_count();
    init_vdso();
    aster_time::VDSO_DATA_UPDATE.call_once(|| Arc::new(update_vdso_instant));
}

/// Return the vdso vmo.
pub(crate) fn vdso_vmo() -> Arc<Vmo> {
    VDSO.get().unwrap().vmo().clone()
}
