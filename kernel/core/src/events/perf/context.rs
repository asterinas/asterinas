// SPDX-License-Identifier: MPL-2.0

//! Per-thread event ownership, cross-CPU synchronization, and scheduler accounting.
//!
//! # Lock Ordering
//!
//! The context state lock precedes the OSTD CPU-local PMU lock.
//! Remote calls are queued and waited on after dropping the state lock and
//! restoring local interrupts.
//! The receiver rechecks the owner before touching hardware.
//! Closing an event never waits on another CPU.

use core::sync::atomic::{AtomicU64, Ordering};

use aster_rights::ReadOp;
use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    irq::{self, DisabledLocalIrqGuard},
    pmu::{Event, EventConfig},
    sync::LocalIrqDisabled,
};

use super::{credentials::AccessCredentials, record::Record, remote};
use crate::{prelude::*, process::Credentials};

/// Number of hardware event slots managed per thread (`CpuCycles` and `Instructions`).
const MAX_THREAD_EVENTS: usize = 2;
/// Limit retries when a remote operation races CPU migration.
const MAX_OWNER_ATTEMPTS: usize = 8;

/// Global monotonically increasing `perf_event` ID generator.
static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

/// Specification for creating a per-thread hardware counting event.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EventSpec {
    /// The event and privilege levels to count.
    pub config: EventConfig,
    /// The CPU on which this event may count, or `None` to follow the thread.
    pub cpu: Option<CpuId>,
    /// Leaves the event disabled when attached.
    pub is_initially_disabled: bool,
    /// Enables the event when the target next successfully executes a program.
    pub should_enable_on_exec: bool,
}

/// Handle identifying an allocated event slot within a [`PerfContext`].
#[derive(Clone, Copy, Debug)]
pub(super) struct Ticket {
    slot: usize,
    id: u64,
}

impl Ticket {
    /// Returns the event's unique identifier.
    pub(super) fn id(self) -> u64 {
        self.id
    }
}

/// Point-in-time counter and timing values for a thread event.
#[derive(Clone, Copy, Debug)]
pub(super) struct Snapshot {
    /// Cumulative event count since creation or the last reset.
    pub count: u64,
    /// Time the event was enabled while the target was scheduled.
    pub enabled_ns: u64,
    /// Time the event occupied a running hardware counter.
    pub running_ns: u64,
}

/// Controls for an existing performance event.
#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    /// Enables counting when the target is eligible to run.
    Enable,
    /// Stops counting and preserves the accumulated value.
    Disable,
    /// Clears the accumulated count and preserves the timing values.
    Reset,
}

/// Per-thread events and their accumulated measurements.
///
/// Each context holds at most one event of each [`Event`] type.
/// Creating, reading, or controlling an event may wait for its owner CPU.
/// Call them from a task context that can sleep, with any locks needed by the owner operation released.
#[derive(Debug)]
pub(crate) struct PerfContext {
    state: SpinLock<State, LocalIrqDisabled>,
}

impl PerfContext {
    /// Creates an empty `PerfContext` for a newly spawned thread.
    pub(crate) fn new() -> Self {
        Self {
            state: SpinLock::new(State {
                task: TaskState::OffCpu,
                access_epoch: Some(0),
                records: [None, None],
            }),
        }
    }

    /// Creates and attaches a new hardware event to this thread context.
    ///
    /// Returns `EACCES` if the credentials changed in a way that invalidates
    /// the authorization associated with `access_epoch`.
    /// Returns `EBUSY` if this event type is already attached.
    pub(super) fn create(self: &Arc<Self>, spec: EventSpec, access_epoch: u64) -> Result<Ticket> {
        let id = NEXT_EVENT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
                cur.checked_add(1)
            })
            .map_err(|_| errno!(ENOSPC, "PMU event ID space exhausted"))?;

        let slot = match spec.config.event() {
            Event::CpuCycles => 0,
            Event::Instructions => 1,
        };
        let mut record = Box::new(Record::new(spec, id));
        self.on_owner(move |state, irq_guard| {
            if state.task == TaskState::Exited {
                return Err(errno!(ESRCH, "the target thread has exited"));
            }
            if Some(access_epoch) != state.access_epoch {
                return Err(errno!(EACCES, "the PMU target's credentials changed"));
            }
            if let Some(mut closed) = state.records[slot].take_if(|record| record.is_closed()) {
                closed.stop(read_now_ns(), irq_guard);
            }
            if state.records[slot].is_some() {
                return Err(errno!(
                    EBUSY,
                    "one event of each type per thread is supported"
                ));
            }
            if state.task.owner().is_some() {
                record
                    .sync_on_cpu(read_now_ns(), irq_guard)
                    .map_err(super::to_error)?;
            }
            state.records[slot] = Some(record);
            Ok(Ticket { slot, id })
        })
    }

    /// Samples the requested event on its owner CPU without stopping it.
    ///
    /// If the thread is off CPU or has exited, returns its accumulated values.
    /// Returns `EBADF` for a closed or stale event, and propagates counter failures.
    pub(super) fn read(self: &Arc<Self>, ticket: Ticket) -> Result<Snapshot> {
        self.on_owner(move |state, irq_guard| {
            let task_active = state.task.owner().is_some();
            Self::record_mut(state, ticket)?.read(read_now_ns(), task_active, irq_guard)
        })
    }

    /// Executes an `Enable`, `Disable`, or `Reset` control operation on `ticket`.
    ///
    /// Enabling a revoked event returns `EACCES`.
    /// Enabling an event after the target exits returns `ESRCH`.
    pub(super) fn control(self: &Arc<Self>, ticket: Ticket, operation: Operation) -> Result<()> {
        self.on_owner(move |state, irq_guard| {
            let task = state.task;
            let record = Self::record_mut(state, ticket)?;
            if task == TaskState::Exited && matches!(operation, Operation::Enable) {
                return Err(errno!(ESRCH, "the target thread has exited"));
            }
            record.control(operation, task.owner().is_some(), read_now_ns(), irq_guard)
        })
    }

    /// Closes the event, deferring any live counter cleanup to its owner CPU.
    pub(super) fn close(&self, ticket: Ticket) {
        let mut state = self.state.lock();
        let Ok(record) = Self::record_mut(&mut state, ticket) else {
            return;
        };
        record.close();
        if record.can_remove() {
            state.records[ticket.slot] = None;
        }
    }

    /// Stops and settles all active counters before the thread is scheduled out.
    ///
    /// Call this on the outgoing thread's CPU before switching away from it.
    pub(crate) fn on_schedule_out(&self, irq_guard: &DisabledLocalIrqGuard) {
        let mut state = self.state.lock();
        if state.records.iter().any(Option::is_some) {
            let now_ns = read_now_ns();
            for slot in &mut state.records {
                if let Some(record) = slot.as_mut() {
                    record.on_schedule_out(now_ns, irq_guard);
                    if record.can_remove() {
                        *slot = None;
                    }
                }
            }
        }
        if state.task != TaskState::Exited {
            state.task = TaskState::OffCpu;
        }
    }

    /// Starts or resumes eligible counters after the thread is scheduled in on the current CPU.
    pub(crate) fn on_schedule_in(&self, irq_guard: &DisabledLocalIrqGuard) {
        let mut state = self.state.lock();
        if state.task != TaskState::Exited {
            state.task = TaskState::OnCpu(irq_guard.current_cpu());
            Self::sync(&mut state, irq_guard);
        }
    }

    /// Samples active counters and processes pending closes on timer ticks.
    pub(crate) fn on_tick(&self) {
        let irq_guard = irq::disable_local();
        let mut state = self.state.lock();
        if state.task.owner() == Some(irq_guard.current_cpu()) {
            Self::sync(&mut state, &irq_guard);
        }
    }

    /// Stops all counters and freezes final snapshots when the thread exits.
    pub(crate) fn on_exit(&self) {
        let irq_guard = irq::disable_local();
        let mut state = self.state.lock();
        state.task = TaskState::Exited;
        if !state.records.iter().any(Option::is_some) {
            return;
        }
        let now_ns = read_now_ns();
        for record in state.records.iter_mut().flatten() {
            record.on_exit(now_ns, &irq_guard);
        }
    }

    /// Returns the target credential generation before authorizing attachment.
    ///
    /// Pass the same generation to [`Self::create`] after checking permission.
    /// An exhausted generation space prevents new attachments.
    pub(crate) fn access_epoch(&self) -> Option<u64> {
        self.state.lock().access_epoch
    }

    /// Preserves the authorized count across a credential update.
    ///
    /// Event files can be shared with another process,
    /// so an update that invalidates the previous authorization revokes existing events.
    /// Revoked events retain their accumulated counts.
    /// The caller runs this on the target thread, and `update` must not sleep.
    pub(crate) fn update_credentials<T>(
        &self,
        credentials: &Credentials<ReadOp>,
        update: impl FnOnce() -> T,
    ) -> T {
        let irq_guard = irq::disable_local();
        let mut state = self.state.lock();
        let before = AccessCredentials::read(credentials);
        let has_events = state
            .records
            .iter()
            .flatten()
            .any(|record| !record.is_revoked());
        if has_events {
            let now_ns = read_now_ns();
            for record in state
                .records
                .iter_mut()
                .flatten()
                .filter(|record| !record.is_revoked())
            {
                record.on_schedule_out(now_ns, &irq_guard);
            }
        }

        let result = update();
        if before.invalidated_by(&AccessCredentials::read(credentials)) {
            // Also invalidate attachments authorized before the change. Check
            // the final credentials even if the original setter returned an error.
            state.access_epoch = state.access_epoch.and_then(|epoch| epoch.checked_add(1));
            if has_events {
                let now_ns = read_now_ns();
                for record in state
                    .records
                    .iter_mut()
                    .flatten()
                    .filter(|record| !record.is_revoked())
                {
                    record.revoke(now_ns, &irq_guard);
                }
            }
        } else if has_events && state.task.owner() == Some(irq_guard.current_cpu()) {
            Self::sync(&mut state, &irq_guard);
        }
        result
    }

    /// Activates `enable_on_exec` events after `execve(2)` succeeds.
    pub(crate) fn on_exec(&self) {
        let irq_guard = irq::disable_local();
        let mut state = self.state.lock();
        if !state.records.iter().any(Option::is_some) {
            return;
        }
        for record in state.records.iter_mut().flatten() {
            record.on_exec();
        }
        if state.task.owner() == Some(irq_guard.current_cpu()) {
            Self::sync(&mut state, &irq_guard);
        }
    }

    fn on_owner<T: Send + 'static, F>(self: &Arc<Self>, mut operation: F) -> Result<T>
    where
        F: FnOnce(&mut State, &DisabledLocalIrqGuard) -> Result<T> + Send + 'static,
    {
        for _ in 0..MAX_OWNER_ATTEMPTS {
            let cpu = {
                let irq_guard = irq::disable_local();
                let mut state = self.state.lock();
                match state.task.owner() {
                    Some(cpu) if cpu != irq_guard.current_cpu() => cpu,
                    _ => return operation(&mut state, &irq_guard),
                }
            };
            let context = self.clone();
            match remote::call_on_cpu(cpu, move || {
                let irq_guard = irq::disable_local();
                let mut state = context.state.lock();
                // A scheduled-out context has no live hardware to sample.
                // Retry only when ownership moved to a different CPU.
                if state.task.owner().is_some_and(|owner| owner != cpu) {
                    Err(operation)
                } else {
                    Ok(operation(&mut state, &irq_guard))
                }
            })? {
                Ok(result) => return result,
                Err(retry) => operation = retry,
            }
        }
        Err(errno!(EAGAIN, "the PMU target kept migrating between CPUs"))
    }

    fn record_mut(state: &mut State, ticket: Ticket) -> Result<&mut Record> {
        state
            .records
            .get_mut(ticket.slot)
            .and_then(Option::as_deref_mut)
            .filter(|record| record.id() == ticket.id && !record.is_closed())
            .ok_or_else(|| errno!(EBADF, "the PMU event no longer exists"))
    }

    fn sync(state: &mut State, irq_guard: &DisabledLocalIrqGuard) {
        if !state.records.iter().any(Option::is_some) {
            return;
        }
        let now_ns = read_now_ns();
        for slot in &mut state.records {
            if let Some(record) = slot.as_mut() {
                // A retired counter can be occupied until its next cleanup retry.
                let _ = record.sync_on_cpu(now_ns, irq_guard);
                if record.can_remove() {
                    *slot = None;
                }
            }
        }
    }
}

#[derive(Debug)]
struct State {
    task: TaskState,
    access_epoch: Option<u64>,
    records: [Option<Box<Record>>; MAX_THREAD_EVENTS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    OffCpu,
    OnCpu(CpuId),
    Exited,
}

impl TaskState {
    fn owner(self) -> Option<CpuId> {
        match self {
            Self::OnCpu(cpu) => Some(cpu),
            Self::OffCpu | Self::Exited => None,
        }
    }
}

fn read_now_ns() -> u64 {
    aster_time::read_monotonic_time()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(ktest)]
mod tests {
    use aster_rights::FullOp;
    use ostd::{pmu::CountingScope, prelude::ktest};

    use super::*;
    use crate::process::credentials::Uid;

    #[ktest]
    fn a_pending_attachment_is_rejected_after_credentials_change() {
        let context = Arc::new(PerfContext::new());
        let old_epoch = context.access_epoch().unwrap();
        let spec = EventSpec {
            config: EventConfig::new(Event::Instructions, CountingScope::User),
            cpu: None,
            is_initially_disabled: true,
            should_enable_on_exec: false,
        };

        let credentials = Credentials::<FullOp>::new_root();
        let read_only = credentials.dup().restrict::<ReadOp>();
        context
            .update_credentials(&read_only, || credentials.set_uid(Uid::new_root()))
            .unwrap();
        assert_eq!(context.access_epoch(), Some(old_epoch));

        context
            .update_credentials(&read_only, || credentials.set_uid(Uid::new(1000)))
            .unwrap();
        assert_eq!(
            context.create(spec, old_epoch).unwrap_err().error(),
            Errno::EACCES
        );

        let new_epoch = context.access_epoch().unwrap();
        let ticket = context.create(spec, new_epoch).unwrap();
        context.close(ticket);
    }
}
