// SPDX-License-Identifier: MPL-2.0

//! CPU-local PMU resources for counting CPU cycles and retired instructions.
//!
//! A [`CounterHandle`] represents one exclusive hardware counter allocation on
//! a specific CPU.
//! All hardware register operations require a [`DisabledLocalIrqGuard`] on the owning CPU.
//! Dropping a handle retires the allocation on its owner CPU. A remote drop
//! leaves cleanup to the owner CPU timer or its next allocation.
//!
//! The upper kernel layer owns per-task attribution and must stop and release
//! active handles before migrating a task across CPUs.

mod accounting;
mod local;
mod types;

pub use types::{BackendKind, CountingScope, Event, EventConfig, PmuCaps, PmuError, Result};

use self::local::{
    LOCAL, Record, collect_locked, collect_retired, refresh_collection_state, with_record,
};
#[cfg(target_arch = "x86_64")]
use crate::arch::pmu as backend;
#[cfg(not(target_arch = "x86_64"))]
#[path = "unsupported.rs"]
mod backend;

use crate::{
    cpu::{CpuId, PinCurrentCpu},
    irq::{self, DisabledLocalIrqGuard},
};

/// Whether this build includes a hardware PMU backend.
///
/// Use [`capabilities`] to check which events are available on a particular CPU.
pub const HAS_BACKEND: bool = backend::HAS_BACKEND;

/// The number of logical event slots managed per CPU.
const MAX_EVENTS: usize = 2;

/// An exclusive CPU-local hardware counter allocation.
#[must_use]
#[derive(Debug)]
pub struct CounterHandle {
    cpu: CpuId,
    slot: usize,
    generation: u64,
}

/// Initializes and caches the PMU capabilities of the current CPU.
pub fn init_on_cpu() -> Result<PmuCaps> {
    let irq_guard = irq::disable_local();
    let local = LOCAL.get_on_cpu(irq_guard.current_cpu());
    if let Some(caps) = local.lock().caps.as_ref() {
        return caps
            .as_ref()
            .map(backend::Caps::summary)
            .map_err(|&err| err);
    }

    // Only the owner CPU initializes its PMU. The IRQ guard serializes
    // initialization while remote capability lookups still see `None`.
    let detected = backend::probe();
    let caps = detected
        .as_ref()
        .map(backend::Caps::summary)
        .map_err(|&err| err);
    local.lock().caps = Some(detected);
    if caps.is_ok() {
        crate::timer::register_callback_on_cpu(collect_retired);
    }
    caps
}

/// Returns the cached PMU capabilities of `cpu` without cross-CPU register access.
pub fn capabilities(cpu: CpuId) -> Result<PmuCaps> {
    LOCAL
        .get_on_cpu(cpu)
        .lock()
        .capabilities()
        .map(|caps| caps.summary())
}

/// Allocates and configures a stopped hardware counter on the current CPU.
///
/// Each CPU has one managed allocation slot for cycles and one for instructions.
/// A stopped handle reserves its slot until it is released.
pub fn configure(config: EventConfig, irq_guard: &DisabledLocalIrqGuard) -> Result<CounterHandle> {
    let cpu = irq_guard.current_cpu();
    let mut local = LOCAL.get_on_cpu(cpu).lock();
    collect_locked(&mut local, cpu);
    local.capabilities()?;

    let slot = match config.event() {
        Event::CpuCycles => 0,
        Event::Instructions => 1,
    };
    if local.records[slot].is_some() {
        return Err(PmuError::NoCounter);
    }

    let generation = local.generations[slot]
        .checked_add(1)
        .ok_or(PmuError::NoCounter)?;
    local.generations[slot] = generation;

    let used = local
        .records
        .each_ref()
        .map(|record| record.as_ref().map(|record| record.token.index()));

    let (token, initial_raw) = backend::configure(config, local.capabilities()?, &used)?;

    let width_bits = token.width_bits();
    local.records[slot] = Some(Record {
        generation,
        token,
        is_running: false,
        is_retired: false,
        fault: None,
        total: 0,
        last: initial_raw & accounting::mask(width_bits),
    });

    Ok(CounterHandle {
        cpu,
        slot,
        generation,
    })
}

/// Starts or resumes a stopped counter on the current CPU.
pub fn start(handle: &CounterHandle, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    with_record(handle, irq_guard, |record| {
        if let Some(err) = record.fault {
            return Err(err);
        }
        if record.is_running {
            return Ok(());
        }

        let initial_raw = record.token.start();
        record.last = initial_raw & accounting::mask(record.token.width_bits());
        record.is_running = true;
        Ok(())
    })
}

/// Reads a non-consuming cumulative count from a counter on the current CPU.
///
/// The caller must observe a running counter before it advances by
/// `2^width` events, where `width` is returned by [`counter_width`].
/// Observations include successful [`start`], `read`, and [`stop`] calls.
/// The modular hardware delta cannot detect a full wrap between observations.
pub fn read(handle: &CounterHandle, irq_guard: &DisabledLocalIrqGuard) -> Result<u64> {
    with_record(handle, irq_guard, observe)
}

/// Stops the hardware counter and returns its final cumulative count.
///
/// The observation interval must satisfy the same bound as [`read`].
pub fn stop(handle: &CounterHandle, irq_guard: &DisabledLocalIrqGuard) -> Result<u64> {
    with_record(handle, irq_guard, |record| {
        if !record.is_running {
            return record.fault.map_or(Ok(record.total), Err);
        }
        // Always stop the hardware, including after an observation error.
        let raw = record.token.stop();
        record.is_running = false;
        if let Some(err) = record.fault {
            return Err(err);
        }
        accumulate(record, raw)
    })
}

/// Releases a counter allocation on the current CPU, stopping it first if needed.
///
/// The handle is consumed. If called on another CPU, returns [`PmuError::WrongCpu`]
/// and defers release to the owner CPU.
pub fn release(handle: CounterHandle, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    if irq_guard.current_cpu() != handle.cpu {
        return Err(PmuError::WrongCpu);
    }

    let mut local = LOCAL.get_on_cpu(handle.cpu).lock();
    let record = local.records[handle.slot]
        .as_ref()
        .ok_or(PmuError::InvalidHandle)?;
    if record.generation != handle.generation {
        return Err(PmuError::InvalidHandle);
    }

    record.token.release();
    local.records[handle.slot] = None;
    refresh_collection_state(&local, handle.cpu);
    // The record is gone, so Drop would only acquire LOCAL again.
    core::mem::forget(handle);
    Ok(())
}

/// Returns the hardware bit width of a valid counter allocation.
pub fn counter_width(handle: &CounterHandle, irq_guard: &DisabledLocalIrqGuard) -> Result<u8> {
    with_record(handle, irq_guard, |record| Ok(record.token.width_bits()))
}

impl Drop for CounterHandle {
    fn drop(&mut self) {
        let irq_guard = irq::disable_local();
        {
            let mut local = LOCAL.get_on_cpu(self.cpu).lock();
            let Some(record) = local.records[self.slot].as_mut() else {
                return;
            };
            if record.generation != self.generation {
                return;
            }
            record.is_retired = true;
            if self.cpu == irq_guard.current_cpu() {
                collect_locked(&mut local, self.cpu);
            } else {
                refresh_collection_state(&local, self.cpu);
            }
        }
        // Never wait in Drop: it can execute under a file-table lock or with IRQs disabled.
        // The owner CPU's timer or its next allocation handles a remote drop.
    }
}

fn observe(record: &mut Record) -> Result<u64> {
    if let Some(err) = record.fault {
        return Err(err);
    }
    if !record.is_running {
        return Ok(record.total);
    }

    let raw = record.token.read();
    accumulate(record, raw)
}

fn accumulate(record: &mut Record, raw: u64) -> Result<u64> {
    let result = accounting::accumulate(
        &mut record.total,
        &mut record.last,
        raw,
        record.token.width_bits(),
    )
    .ok_or(PmuError::Overflow);
    if let Err(err) = result {
        record.fault = Some(err);
    }
    result
}
