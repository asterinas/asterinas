// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_set::BTreeSet, sync::Arc};
use core::{
    cmp::Ordering,
    sync::atomic::{AtomicU64, Ordering as SyncOrdering},
};

use ostd::task::{
    scheduler::{EnqueueFlags, UpdateFlags},
    Task,
};

use super::{CurrentRuntime, SchedAttr, SchedClassRq};
use crate::thread::AsThread;

#[derive(Debug)]
struct TaskLagged {
    task: Arc<Task>,
    lag: u64,
    id: u64,
}

impl PartialEq for TaskLagged {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl Eq for TaskLagged {}

impl Ord for TaskLagged {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.lag.cmp(&other.lag) {
            Ordering::Equal => self.id.cmp(&other.id),
            ordering => ordering,
        }
    }
}

impl PartialOrd for TaskLagged {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub(super) struct EarliestDeadlineRq {
    current: Option<TaskLagged>,
    eligible: BTreeSet<TaskLagged>,
    counter: u64,
}

#[derive(Debug)]
pub(super) struct EarliestDeadlineAttr {
    lag: AtomicU64,
}

impl EarliestDeadlineAttr {
    pub(super) fn new() -> Self {
        Self {
            lag: AtomicU64::new(0),
        }
    }
}

impl EarliestDeadlineRq {
    pub(super) fn new() -> Self {
        Self {
            current: None,
            eligible: BTreeSet::default(),
            counter: 0,
        }
    }
}

impl SchedClassRq for EarliestDeadlineRq {
    fn enqueue(&mut self, task: Arc<Task>, flags: Option<EnqueueFlags>) {
        let lag = if flags == Some(EnqueueFlags::Wake) {
            task.as_thread()
                .unwrap()
                .sched_attr()
                .eevdf
                .lag
                .load(SyncOrdering::Acquire)
        } else {
            // The lag must be high enough for the new task to go last.
            match self.eligible.last() {
                Some(last) => last.lag + 1,
                None => 0,
            }
        };

        let id = self.counter;
        self.eligible.insert(TaskLagged { task, lag, id });
        self.counter += 1;
    }

    fn len(&self) -> usize {
        self.eligible.len()
    }

    fn is_empty(&self) -> bool {
        self.eligible.is_empty()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        self.current = self.eligible.pop_first();
        self.current.as_ref().map(|tl| tl.task.clone())
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        let Some(mut current) = self.current.take() else {
            // There's no current task.
            return !self.is_empty();
        };

        match flags {
            UpdateFlags::Tick => {
                current.lag += rt.delta;
                if let Some(first) = self.eligible.first() {
                    if first.lag < current.lag {
                        // The eligible task *will* preempt the current one.
                        self.eligible.insert(current);
                        true
                    } else {
                        // There is an eligible task, but it should not preempt
                        // the current one.
                        self.current = Some(current);
                        false
                    }
                } else {
                    // There's no eligible task.
                    self.current = Some(current);
                    false
                }
            }
            UpdateFlags::Yield => {
                current.lag += rt.delta;
                if let Some(mut first) = self.eligible.pop_first() {
                    // Since `current` is yielding CPU, guarantee that `first`
                    // will preempt it.
                    first.lag = first.lag.min(current.lag.saturating_sub(1));
                    self.eligible.insert(current);
                    self.eligible.insert(first);
                    true
                } else {
                    // There's no eligible task.
                    self.current = Some(current);
                    false
                }
            }
            UpdateFlags::Wait => {
                if let Some(current) = &self.current {
                    current
                        .task
                        .as_thread()
                        .unwrap()
                        .sched_attr()
                        .eevdf
                        .lag
                        .store(current.lag + rt.delta, SyncOrdering::Relaxed);
                }
                attr.eevdf
                    .lag
                    .store(current.lag + rt.delta, SyncOrdering::Relaxed);
                !self.is_empty()
            }
            UpdateFlags::Exit => !self.is_empty(),
        }
    }
}
