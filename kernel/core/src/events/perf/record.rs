// SPDX-License-Identifier: MPL-2.0

//! Counter ownership and accounting for one logical thread event.

use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    irq::DisabledLocalIrqGuard,
    pmu::{self, CounterHandle, EventConfig, PmuError},
};

use super::{EventSpec, Operation, Snapshot, activity::EventActivity, enabled_time::ActiveTime};
use crate::prelude::*;

#[derive(Debug)]
pub(super) struct Record {
    _activity: EventActivity,
    id: u64,
    config: EventConfig,
    cpu: Option<CpuId>,
    flags: EventFlags,
    fault: Option<PmuError>,
    base: CountAndTime,
    enabled_time: ActiveTime,
    live: Option<RunningSegment>,
}

impl Record {
    pub(super) fn new(spec: EventSpec, id: u64) -> Self {
        let mut flags = EventFlags::empty();
        flags.set(EventFlags::ENABLED, !spec.is_initially_disabled);
        flags.set(EventFlags::ENABLE_ON_EXEC, spec.should_enable_on_exec);

        Self {
            _activity: EventActivity::new(),
            id,
            config: spec.config,
            cpu: spec.cpu,
            flags,
            fault: None,
            base: CountAndTime::default(),
            enabled_time: ActiveTime::default(),
            live: None,
        }
    }

    pub(super) fn id(&self) -> u64 {
        self.id
    }

    pub(super) fn is_closed(&self) -> bool {
        self.flags.contains(EventFlags::CLOSED)
    }

    pub(super) fn is_revoked(&self) -> bool {
        self.flags.contains(EventFlags::REVOKED)
    }

    pub(super) fn can_remove(&self) -> bool {
        self.is_closed() && self.live.is_none()
    }

    pub(super) fn close(&mut self) {
        self.flags.insert(EventFlags::CLOSED);
    }

    pub(super) fn on_exec(&mut self) {
        if self.flags.contains(EventFlags::ENABLE_ON_EXEC) && !self.is_closed() {
            self.flags.remove(EventFlags::ENABLE_ON_EXEC);
            self.flags.insert(EventFlags::ENABLED);
        }
    }

    pub(super) fn on_schedule_out(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) {
        self.update_enabled(now_ns, false);
        self.stop(now_ns, irq_guard);
    }

    pub(super) fn on_exit(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) {
        self.disable(now_ns);
        self.stop(now_ns, irq_guard);
    }

    pub(super) fn revoke(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) {
        self.on_exit(now_ns, irq_guard);
        self.flags.remove(EventFlags::ENABLE_ON_EXEC);
        self.flags.insert(EventFlags::REVOKED);
    }

    pub(super) fn read(
        &mut self,
        now_ns: u64,
        task_active: bool,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<Snapshot> {
        self.update_enabled(now_ns, task_active);
        self.sample(now_ns, irq_guard);
        self.snapshot()
    }

    pub(super) fn control(
        &mut self,
        operation: Operation,
        task_active: bool,
        now_ns: u64,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<()> {
        match operation {
            Operation::Disable => self.on_exit(now_ns, irq_guard),
            Operation::Enable => {
                if self.is_revoked() {
                    return Err(errno!(EACCES, "the PMU target's credentials changed"));
                }
                self.check_fault()?;
                let was_enabled = self.flags.contains(EventFlags::ENABLED);
                self.flags.insert(EventFlags::ENABLED);
                if task_active && let Err(error) = self.sync_on_cpu(now_ns, irq_guard) {
                    if !was_enabled {
                        self.disable(now_ns);
                    }
                    return Err(super::to_error(error));
                }
            }
            Operation::Reset => self.reset(now_ns, irq_guard)?,
        }
        self.check_fault()
    }

    // A busy counter can be retried on the next tick.
    // Other errors are latched and prevent restarting the event.
    pub(super) fn sync_on_cpu(
        &mut self,
        now_ns: u64,
        irq_guard: &DisabledLocalIrqGuard,
    ) -> pmu::Result<()> {
        self.update_enabled(now_ns, true);
        if self.flags.contains(EventFlags::ENABLED)
            && !self.is_closed()
            && self.fault.is_none()
            && self.cpu.is_none_or(|cpu| cpu == irq_guard.current_cpu())
        {
            if let Err(error) = self.start(now_ns, irq_guard) {
                if error != PmuError::NoCounter {
                    self.fault = Some(error);
                }
                return Err(error);
            }
            self.sample(now_ns, irq_guard);
        } else {
            self.stop(now_ns, irq_guard);
        }
        self.fault.map_or(Ok(()), Err)
    }

    pub(super) fn stop(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) {
        let Some(segment) = self.live.take() else {
            return;
        };
        let interval_res = segment.check_interval(now_ns);
        let stop_res = pmu::stop(&segment.handle, irq_guard);
        let settled = interval_res
            .and(stop_res)
            .and_then(|total| segment.add_to(self.base, total, now_ns));
        let released = pmu::release(segment.handle, irq_guard);
        if let Err(err) = settled.map(|base| self.base = base).and(released) {
            self.fault.get_or_insert(err);
        }
    }

    fn disable(&mut self, now_ns: u64) {
        self.update_enabled(now_ns, false);
        self.flags.remove(EventFlags::ENABLED);
    }

    fn update_enabled(&mut self, now_ns: u64, task_active: bool) {
        // A CPU filter can stop counting while the event remains enabled.
        let active = task_active && self.flags.contains(EventFlags::ENABLED) && !self.is_closed();
        if self.enabled_time.update(now_ns, active).is_none() {
            self.fault.get_or_insert(PmuError::Overflow);
        }
    }

    fn reset(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        self.check_fault()?;
        self.sample(now_ns, irq_guard);
        self.check_fault()?;
        // RESET affects the count, not the enabled or running times.
        self.base.count = 0;
        if let Some(segment) = self.live.as_mut() {
            segment.count_at_reset = segment.total;
        }
        Ok(())
    }

    fn start(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) -> pmu::Result<()> {
        if self.live.is_some() {
            return Ok(());
        }
        let handle = pmu::configure(self.config, irq_guard)?;
        let width_bits = pmu::counter_width(&handle, irq_guard)?;
        pmu::start(&handle, irq_guard)?;
        self.live = Some(RunningSegment {
            handle,
            width_bits,
            since_ns: now_ns,
            last_observed_ns: now_ns,
            total: 0,
            count_at_reset: 0,
        });
        Ok(())
    }

    fn sample(&mut self, now_ns: u64, irq_guard: &DisabledLocalIrqGuard) {
        let Some(segment) = self.live.as_mut() else {
            return;
        };
        match segment
            .check_interval(now_ns)
            .and_then(|()| pmu::read(&segment.handle, irq_guard))
        {
            Ok(total) => {
                segment.total = total;
                segment.last_observed_ns = now_ns;
            }
            Err(err) => {
                self.fault.get_or_insert(err);
                self.stop(now_ns, irq_guard);
            }
        }
    }

    fn snapshot(&self) -> Result<Snapshot> {
        self.check_fault()?;
        let values = match self.live.as_ref() {
            Some(segment) => segment
                .add_to(self.base, segment.total, segment.last_observed_ns)
                .map_err(super::to_error)?,
            None => self.base,
        };

        Ok(Snapshot {
            count: values.count,
            enabled_ns: self.enabled_time.total(),
            running_ns: values.running_ns,
        })
    }

    fn check_fault(&self) -> Result<()> {
        self.fault.map_or(Ok(()), |err| Err(super::to_error(err)))
    }
}

bitflags! {
    /// Logical event state, tracked separately from hardware counter ownership.
    #[derive(Debug)]
    struct EventFlags: u8 {
        const REVOKED = 1 << 0;
        const ENABLED = 1 << 1;
        const ENABLE_ON_EXEC = 1 << 2;
        const CLOSED = 1 << 3;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CountAndTime {
    count: u64,
    running_ns: u64,
}

#[derive(Debug)]
struct RunningSegment {
    handle: CounterHandle,
    width_bits: u8,
    since_ns: u64,
    last_observed_ns: u64,
    total: u64,
    count_at_reset: u64,
}

impl RunningSegment {
    fn check_interval(&self, now_ns: u64) -> core::result::Result<(), PmuError> {
        if super::observation::interval_is_safe(self.width_bits, self.last_observed_ns, now_ns) {
            Ok(())
        } else {
            Err(PmuError::Overflow)
        }
    }

    fn add_to(&self, base: CountAndTime, total: u64, now_ns: u64) -> pmu::Result<CountAndTime> {
        let count = total
            .checked_sub(self.count_at_reset)
            .and_then(|delta| base.count.checked_add(delta))
            .ok_or(PmuError::Overflow)?;
        let running_ns = now_ns
            .checked_sub(self.since_ns)
            .and_then(|delta| base.running_ns.checked_add(delta))
            .ok_or(PmuError::Overflow)?;
        Ok(CountAndTime { count, running_ns })
    }
}
