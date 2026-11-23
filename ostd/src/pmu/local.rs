// SPDX-License-Identifier: MPL-2.0

//! Per-CPU PMU allocation records and deferred retirement.
//!
//! The owner CPU holds `LOCAL` across a register transaction and the
//! corresponding record update. Other CPUs may inspect capabilities or mark
//! an allocation retired. Register accesses here are synchronous local
//! operations. Never wait for another CPU or take a scheduler lock under
//! `LOCAL`. An upper-layer event lock, when present, precedes `LOCAL`.

use core::sync::atomic::{AtomicBool, Ordering};

use super::{CounterHandle, MAX_EVENTS, PmuError, Result, backend};
use crate::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    sync::{LocalIrqDisabled, SpinLock},
};

cpu_local! {
    static NEEDS_COLLECTION: AtomicBool = AtomicBool::new(false);
    pub(super) static LOCAL: SpinLock<LocalPmu, LocalIrqDisabled> = SpinLock::new(LocalPmu {
        caps: None,
        generations: [0; MAX_EVENTS],
        records: [None, None],
    });
}

pub(super) struct LocalPmu {
    pub(super) caps: Option<Result<backend::Caps>>,
    pub(super) generations: [u64; MAX_EVENTS],
    pub(super) records: [Option<Record>; MAX_EVENTS],
}

impl LocalPmu {
    pub(super) fn capabilities(&self) -> Result<&backend::Caps> {
        self.caps
            .as_ref()
            .ok_or(PmuError::Unsupported)?
            .as_ref()
            .map_err(|&error| error)
    }
}

#[derive(Debug)]
pub(super) struct Record {
    pub(super) generation: u64,
    pub(super) token: backend::Token,
    pub(super) is_running: bool,
    pub(super) is_retired: bool,
    pub(super) fault: Option<PmuError>,
    pub(super) total: u64,
    pub(super) last: u64,
}

pub(super) fn with_record<T>(
    handle: &CounterHandle,
    irq_guard: &DisabledLocalIrqGuard,
    operation_fn: impl FnOnce(&mut Record) -> Result<T>,
) -> Result<T> {
    if irq_guard.current_cpu() != handle.cpu {
        return Err(PmuError::WrongCpu);
    }
    let mut local = LOCAL.get_on_cpu(handle.cpu).lock();
    let record = local.records[handle.slot]
        .as_mut()
        .ok_or(PmuError::InvalidHandle)?;
    if record.generation != handle.generation || record.is_retired {
        return Err(PmuError::InvalidHandle);
    }
    operation_fn(record)
}

/// Releases retired counter tokens on the current CPU.
pub(super) fn collect_retired() {
    // This is an advisory fast path. In task context, migration can at most
    // defer cleanup until the next owner-CPU timer callback; configure also
    // collects under LOCAL's lock before allocating a counter.
    if !NEEDS_COLLECTION
        .get_on_cpu(CpuId::current_racy())
        .load(Ordering::Acquire)
    {
        return;
    }
    let irq_guard = irq::disable_local();
    let cpu = irq_guard.current_cpu();
    collect_locked(&mut LOCAL.get_on_cpu(cpu).lock(), cpu);
}

pub(super) fn collect_locked(local: &mut LocalPmu, cpu: CpuId) {
    for slot in &mut local.records {
        if let Some(record) = slot.as_ref().filter(|record| record.is_retired) {
            record.token.release();
            *slot = None;
        }
    }
    NEEDS_COLLECTION
        .get_on_cpu(cpu)
        .store(false, Ordering::Release);
}

// Called under LOCAL's lock, including when a remote Drop retires a token.
pub(super) fn refresh_collection_state(local: &LocalPmu, cpu: CpuId) {
    let needs_collection = local
        .records
        .iter()
        .flatten()
        .any(|record| record.is_retired);
    NEEDS_COLLECTION
        .get_on_cpu(cpu)
        .store(needs_collection, Ordering::Release);
}
