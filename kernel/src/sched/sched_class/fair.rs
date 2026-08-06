// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};
use core::{
    borrow::Borrow,
    cmp,
    sync::atomic::{AtomicU64, Ordering},
};

use ostd::{
    cpu::{self, CpuId},
    sync::SpinLock,
    task::{
        Task,
        scheduler::{EnqueueFlags, UpdateFlags},
    },
};

use super::{
    CurrentRuntime, SchedClassRq,
    task_group::TaskGroup,
    time::{base_slice_clocks, min_period_clocks},
};
use crate::{
    sched::{Nice, nice::NiceValue},
    thread::{AsThread, Thread},
};

pub(super) const WEIGHT_0: u64 = 1024;
pub(crate) const DEFAULT_CGROUP_WEIGHT: u32 = 100;

const HAS_PENDING: u64 = 1 << (u64::BITS - 1);

pub const fn nice_to_weight(nice: Nice) -> u64 {
    // Calculated by the formula below:
    //
    //     weight = 1024 * 1.25^(-nice)
    //
    // We propose that every increment of the nice value results
    // in 12.5% change of the CPU load weight.
    const FACTOR_NUMERATOR: u64 = 5;
    const FACTOR_DENOMINATOR: u64 = 4;

    const NICE_TO_WEIGHT: [u64; 40] = const {
        let mut ret = [0; 40];

        let mut index = 0;
        let mut nice = NiceValue::MIN.get();
        while nice <= NiceValue::MAX.get() {
            ret[index] = match nice {
                0 => WEIGHT_0,
                nice @ 1.. => {
                    let numerator = FACTOR_DENOMINATOR.pow(nice as u32);
                    let denominator = FACTOR_NUMERATOR.pow(nice as u32);
                    WEIGHT_0 * numerator / denominator
                }
                nice => {
                    let numerator = FACTOR_NUMERATOR.pow((-nice) as u32);
                    let denominator = FACTOR_DENOMINATOR.pow((-nice) as u32);
                    WEIGHT_0 * numerator / denominator
                }
            };
            assert!(ret[index] & HAS_PENDING == 0);

            index += 1;
            nice += 1;
        }
        ret
    };

    NICE_TO_WEIGHT[(nice.value().get() + 20) as usize]
}

/// The scheduling attributes for a FAIR scheduling entity.
///
/// A FAIR entity is either a thread or a task group entity. The structure
/// contains a significant indicator: `vruntime`.
///
/// # `vruntime`
///
/// The vruntime (virtual runtime) is calculated by the formula:
///
///     vruntime += runtime_delta * WEIGHT_0 / weight
///
/// and an entity with a lower vruntime gains a greater privilege to be
/// scheduled, making the whole run queue balanced on vruntime (thus FAIR).
///
/// # Scheduling periods
///
/// Scheduling periods are designed to calculate the time slice for each entity.
///
/// The time slice for each entity is calculated by the formula:
///
///     time_slice = period * weight / total_weight
///
/// where `total_weight` is the sum of the queued entity weights plus the
/// current entity's weight, and [`period`](FairClassRq::period) is calculated
/// regarding the number of runnable entities.
///
/// When the current entity meets the condition below,
/// [`FairClassRq::update_current`] asks the scheduler to preempt the current
/// task.
///
///     period_delta > time_slice
///         || vruntime > rq_min_vruntime + normalized_time_slice
///
/// # The weight update process
///
/// The weight of an entity can be updated outside the runqueue that currently
/// contains it, for example by `sched_setattr` for threads or `cpu.weight` for
/// task groups. This makes it difficult to re-evaluate the runqueue instantly
/// after the update without making each entity point back to its runqueue.
///
/// To handle this problem, we use a `pending_weight` field to store the new weight.
/// When the entity is enqueued, refreshed, or updated as the current entity, we
/// check if the weight needs to be updated since both the old and new weights
/// are needed for re-evaluation.
///
/// To indicate whether the weight needs to be updated, we pack the `weight` field
/// with a bit flag `HAS_PENDING`. When accessing the `weight` field:
///
/// - If the weight does not need to be updated (i.e., `weight & HAS_PENDING == 0`),
///   we simply return the weight.
/// - If the weight needs to be updated (i.e., `weight & HAS_PENDING != 0`), we try to
///   store the new weight into the `weight` field, which shouldn't take too much time
///   since the update frequency is usually relatively low.
/// - After a successful update, we re-evaluate the data of the run queue.
///
/// Most of the time, this mechanism allows the access to the weight lock-free and
/// ensures that only one load is needed.
#[derive(Debug)]
pub struct FairAttr {
    /// Stable entity ID for deterministic tie-breaking in runqueue ordering.
    id: u64,
    // Updates to the `weight` field must be serialized with the `pending_weight` lock.
    weight: AtomicU64,
    pending_weight: SpinLock<u64>,
    vruntime: AtomicU64,
    queued_weight: SpinLock<u64>,
    /// Vruntime offset from the source runqueue's `min_vruntime` during migration.
    migration_lag: SpinLock<Option<u64>>,
}

impl FairAttr {
    pub fn new(nice: Nice) -> Self {
        let weight = nice_to_weight(nice);
        FairAttr {
            id: next_entity_id(),
            weight: weight.into(),
            pending_weight: SpinLock::new(weight),
            vruntime: Default::default(),
            queued_weight: SpinLock::new(weight),
            migration_lag: SpinLock::new(None),
        }
    }

    pub fn from_weight(weight: u64) -> Self {
        Self {
            id: next_entity_id(),
            weight: weight.into(),
            pending_weight: SpinLock::new(weight),
            vruntime: Default::default(),
            queued_weight: SpinLock::new(weight),
            migration_lag: SpinLock::new(None),
        }
    }

    pub fn update(&self, nice: Nice) {
        let mut pending_weight = self.pending_weight.lock();
        *pending_weight = nice_to_weight(nice);
        self.weight.store(
            self.weight.load(Ordering::Relaxed) | HAS_PENDING,
            Ordering::Relaxed,
        );
    }

    pub fn update_weight(&self, weight: u64) {
        let mut pending_weight = self.pending_weight.lock();
        *pending_weight = weight;
        self.weight.store(
            self.weight.load(Ordering::Relaxed) | HAS_PENDING,
            Ordering::Relaxed,
        );
    }

    fn update_vruntime(&self, delta: u64, weight: u64) -> u64 {
        let weight = weight.max(1);
        let delta = delta.saturating_mul(WEIGHT_0) / weight;

        let mut old_vruntime = self.vruntime.load(Ordering::Relaxed);
        loop {
            let new_vruntime = old_vruntime.saturating_add(delta);
            match self.vruntime.compare_exchange_weak(
                old_vruntime,
                new_vruntime,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return new_vruntime,
                Err(current_vruntime) => old_vruntime = current_vruntime,
            }
        }
    }

    fn fetch_weight(&self) -> (u64, u64) {
        // Synchronization is done via the `pending_weight` lock. Therefore, no additional ordering
        // is required to access the atomic variable.
        let weight = self.weight.load(Ordering::Relaxed);
        if weight & HAS_PENDING == 0 {
            return (weight, weight);
        }

        let new_weight = {
            // `pending_weight` always stores the latest weight.
            let pending_weight = self.pending_weight.lock();
            self.weight.store(*pending_weight, Ordering::Relaxed);
            *pending_weight
        };
        let old_weight = weight & !HAS_PENDING;

        // The `vruntime` field is an accumulated value, and we don't update
        // it here.

        (old_weight, new_weight)
    }

    fn vruntime(&self) -> u64 {
        self.vruntime.load(Ordering::Relaxed)
    }

    fn update_vruntime_at_least(&self, vruntime: u64) -> u64 {
        self.vruntime
            .fetch_max(vruntime, Ordering::Relaxed)
            .max(vruntime)
    }

    fn set_vruntime(&self, vruntime: u64) {
        self.vruntime.store(vruntime, Ordering::Relaxed);
    }

    fn save_migration_lag(&self, old_min_vruntime: u64) {
        let lag = self.vruntime().saturating_sub(old_min_vruntime);
        *self.migration_lag.lock() = Some(lag);
    }

    fn take_migration_lag(&self) -> Option<u64> {
        self.migration_lag.lock().take()
    }
}

#[derive(Clone, Debug)]
enum FairEntity {
    Thread(Arc<Task>),
    Group(Arc<TaskGroup>),
}

impl FairEntity {
    fn fair_attr(&self, cpu: CpuId) -> Option<&FairAttr> {
        match self {
            Self::Thread(task) => Some(&task.as_thread()?.sched_attr().fair),
            Self::Group(task_group) => task_group.fair_attr(cpu),
        }
    }
}

/// Wraps a FAIR scheduling entity with its runqueue key.
///
/// This structure provides the ordering key used by the `BTreeSet` in
/// [`FairClassRq`].
#[derive(Clone)]
struct FairQueueItem {
    key: (u64, u64),
    entity: FairEntity,
}

impl FairQueueItem {
    fn new(key: (u64, u64), entity: FairEntity) -> Self {
        Self { key, entity }
    }

    fn key(&self) -> (u64, u64) {
        self.key
    }

    fn entity(&self) -> &FairEntity {
        &self.entity
    }

    fn into_entity(self) -> FairEntity {
        self.entity
    }
}

impl Borrow<(u64, u64)> for FairQueueItem {
    fn borrow(&self) -> &(u64, u64) {
        &self.key
    }
}

impl core::fmt::Debug for FairQueueItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.key())
    }
}

impl PartialEq for FairQueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.key().eq(&other.key())
    }
}

impl Eq for FairQueueItem {}

impl PartialOrd for FairQueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FairQueueItem {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

fn next_entity_id() -> u64 {
    static NEXT_ENTITY_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ENTITY_ID.fetch_add(1, Ordering::Relaxed)
}

/// The per-CPU FAIR runqueue for one task group.
///
/// See [`FairAttr`] for the explanation of vruntimes and scheduling periods.
///
/// The structure contains a `BTreeSet` to store scheduling entities in the
/// runqueue, ensuring efficient lookup of the next entity to run.
#[derive(Debug)]
pub(crate) struct FairClassRq {
    cpu: CpuId,
    /// The task group this runqueue belongs to.
    task_group: Weak<TaskGroup>,
    /// Queued scheduling entities.
    entities: BTreeSet<FairQueueItem>,
    /// Maps entity IDs to keys in `entities`.
    entity_keys: BTreeMap<u64, (u64, u64)>,
    /// The minimum of vruntime in the run queue. Serves as the initial value of
    /// newly-enqueued entities.
    min_vruntime: u64,
    /// Sum of weights of queued entities.
    total_weight: u64,
    /// Total number of queued tasks in this runqueue and all descendant runqueues.
    total_queued_task_count: usize,
}

impl FairClassRq {
    pub(super) fn new(cpu: CpuId, task_group: Weak<TaskGroup>) -> Self {
        Self {
            cpu,
            task_group,
            entities: BTreeSet::new(),
            entity_keys: BTreeMap::new(),
            min_vruntime: 0,
            total_weight: 0,
            total_queued_task_count: 0,
        }
    }

    pub(super) fn task_group(&self) -> Option<Arc<TaskGroup>> {
        self.task_group.upgrade()
    }

    fn min_queued_vruntime(&self) -> Option<u64> {
        self.entities.iter().next().map(|item| item.key().0)
    }

    /// The scheduling period is calculated as the maximum of the following two values:
    ///
    /// 1. The minimum period value, defined by [`min_period_clocks`].
    /// 2. `period = min_granularity * n` where
    ///    `min_granularity = log2(1 + num_cpus) * base_slice_clocks`, and `n`
    ///    is the number of queued entities plus the entity being considered.
    ///
    /// The formula is chosen by 3 principles:
    ///
    /// 1. The scheduling period should reflect the runnable entities and CPUs;
    /// 2. The scheduling period should not be too low to limit the overhead of context switching;
    /// 3. The scheduling period should not be too high to ensure the scheduling latency
    ///    & responsiveness.
    fn period(&self, queued_entity_count: usize) -> u64 {
        let base_slice_clks = base_slice_clocks();
        let min_period_clks = min_period_clocks();
        // `+ 1` means including the entity being considered.
        let period_single_cpu =
            (base_slice_clks * (queued_entity_count + 1) as u64).max(min_period_clks);
        period_single_cpu * u64::from((1 + cpu::num_cpus()).ilog2())
    }

    /// The virtual time slice for each entity in the run queue, measured in
    /// vruntime clocks.
    fn vtime_slice(&self, queued_entity_count: usize) -> u64 {
        self.period(queued_entity_count) / (queued_entity_count + 1) as u64
    }

    /// The time slice for each entity in the run queue, measured in sched clocks.
    fn time_slice(
        &self,
        current_weight: u64,
        queued_weight: u64,
        queued_entity_count: usize,
    ) -> u64 {
        let total_weight = queued_weight.saturating_add(current_weight).max(1);
        self.period(queued_entity_count) * current_weight / total_weight
    }

    fn add_queued_task(&mut self) {
        self.total_queued_task_count = self.total_queued_task_count.saturating_add(1);
    }

    fn remove_queued_task(&mut self) {
        self.total_queued_task_count = self.total_queued_task_count.saturating_sub(1);
    }

    fn update_queued_task_count_upwards(
        &mut self,
        child_group: &Arc<TaskGroup>,
        update_fn: fn(&mut Self),
    ) {
        let Some(self_group) = self.task_group() else {
            return;
        };
        let mut current_group = child_group.clone();
        while let Some(parent_group) = current_group.parent() {
            if Arc::ptr_eq(&parent_group, &self_group) {
                update_fn(self);
                break;
            }

            {
                let mut parent_rq = parent_group.fair_queue(self.cpu).disable_irq().lock();
                update_fn(&mut parent_rq);
            }
            current_group = parent_group;
        }
    }

    /// Enqueues an entity into this runqueue, updating `total_weight`.
    fn enqueue_entity(&mut self, fair_attr: &FairAttr, fair_entity: FairEntity) {
        let key = (fair_attr.vruntime(), fair_attr.id);
        if let Some(old_key) = self.entity_keys.get(&fair_attr.id).copied() {
            self.remove_entity(old_key);
        }

        let weight = fair_attr.fetch_weight().1;
        *fair_attr.queued_weight.lock() = weight;
        let item = FairQueueItem::new(key, fair_entity);
        if let Some(old_item) = self.entities.replace(item) {
            let old_entity = old_item.into_entity();
            if let Some(old_attr) = old_entity.fair_attr(self.cpu) {
                self.total_weight = self
                    .total_weight
                    .saturating_sub(*old_attr.queued_weight.lock());
            }
        }
        self.entity_keys.insert(fair_attr.id, key);
        self.total_weight += weight;
    }

    /// Removes the entity at `key`, updating `total_weight`.
    fn remove_entity(&mut self, key: (u64, u64)) -> Option<FairEntity> {
        let item = self.entities.take(&key)?;
        self.entity_keys.remove(&key.1);
        let fair_entity = item.into_entity();
        if let Some(fair_attr) = fair_entity.fair_attr(self.cpu) {
            self.total_weight = self
                .total_weight
                .saturating_sub(*fair_attr.queued_weight.lock());
        }
        Some(fair_entity)
    }

    fn remove_entity_by_id(&mut self, entity_id: u64) -> Option<FairEntity> {
        let key = self.entity_keys.get(&entity_id).copied()?;
        let fair_entity = self.remove_entity(key);
        if fair_entity.is_none() {
            self.entity_keys.remove(&entity_id);
        }
        fair_entity
    }

    fn has_entity(&self, entity_id: u64) -> bool {
        self.entity_keys.contains_key(&entity_id)
    }

    /// Refreshes a queued entity after its scheduling attributes change.
    pub(super) fn refresh_queued_entity(&mut self, fair_attr: &FairAttr) {
        let Some(fair_entity) = self.remove_entity_by_id(fair_attr.id) else {
            return;
        };
        self.enqueue_entity(fair_attr, fair_entity);
    }

    pub(super) fn total_queued_task_count(&self) -> usize {
        self.total_queued_task_count
    }

    pub(super) fn try_dequeue_task(
        &mut self,
        task: &Arc<Task>,
        task_group: &Arc<TaskGroup>,
    ) -> bool {
        let Some(thread) = task.as_thread() else {
            return false;
        };
        let fair_attr = &thread.sched_attr().fair;

        if self
            .task_group()
            .is_some_and(|self_group| Arc::ptr_eq(task_group, &self_group))
        {
            let Some(queued_entity) = self.remove_entity_by_id(fair_attr.id) else {
                return false;
            };
            drop(queued_entity);
            fair_attr.save_migration_lag(self.min_vruntime);
            self.remove_queued_task();
            return true;
        }

        let leaf_is_empty = {
            let mut leaf_rq = task_group.fair_queue(self.cpu).disable_irq().lock();
            let Some(queued_entity) = leaf_rq.remove_entity_by_id(fair_attr.id) else {
                return false;
            };
            drop(queued_entity);
            fair_attr.save_migration_lag(leaf_rq.min_vruntime);
            leaf_rq.remove_queued_task();
            leaf_rq.is_empty()
        };
        self.update_queued_task_count_upwards(task_group, Self::remove_queued_task);

        if leaf_is_empty {
            let mut current_group = task_group.clone();
            while let Some(parent_group) = current_group.parent() {
                if !current_group
                    .fair_queue(self.cpu)
                    .disable_irq()
                    .lock()
                    .is_empty()
                {
                    break;
                }
                if let Some(self_group) = self.task_group()
                    && Arc::ptr_eq(&parent_group, &self_group)
                {
                    self.dequeue_group_entity(&current_group);
                } else {
                    parent_group
                        .fair_queue(self.cpu)
                        .disable_irq()
                        .lock()
                        .dequeue_group_entity(&current_group);
                }
                current_group = parent_group;
            }
        }

        true
    }

    fn update_current_entity(
        &mut self,
        fair_attr: &FairAttr,
        rt: &CurrentRuntime,
        flags: UpdateFlags,
    ) -> bool {
        let queued_entity = self.remove_entity_by_id(fair_attr.id);
        let was_queued = queued_entity.is_some();
        let weight = fair_attr.fetch_weight().1;
        let vruntime = fair_attr.update_vruntime(rt.delta, weight);
        if let Some(fair_entity) = queued_entity {
            self.enqueue_entity(fair_attr, fair_entity);
        }
        let min_queued_vruntime = self.min_queued_vruntime();
        self.min_vruntime = match min_queued_vruntime {
            Some(min_queued_vruntime) => vruntime.min(min_queued_vruntime),
            None => vruntime,
        };

        if self.is_empty() {
            return false;
        }
        if matches!(
            flags,
            UpdateFlags::Wait | UpdateFlags::Yield | UpdateFlags::Exit
        ) {
            return true;
        }

        let queued_entity_count = self.len().saturating_sub(usize::from(was_queued));
        let queued_weight = self
            .total_weight
            .saturating_sub(if was_queued { weight } else { 0 });
        rt.period_delta > self.time_slice(weight, queued_weight, queued_entity_count)
            || vruntime > self.min_vruntime + self.vtime_slice(queued_entity_count)
    }

    /// Enqueues `child_group`'s entity into this runqueue.
    fn enqueue_group_entity(&mut self, child_group: &Arc<TaskGroup>) -> bool {
        let Some(group_attr) = child_group.fair_attr(self.cpu) else {
            return false; // root has no group entity attributes
        };

        if self.has_entity(group_attr.id) {
            return false; // already active
        }

        let was_empty = self.entities.is_empty();
        group_attr.update_vruntime_at_least(self.min_vruntime);
        self.enqueue_entity(group_attr, FairEntity::Group(child_group.clone()));

        was_empty
    }

    /// Dequeues `child_group`'s entity from this runqueue.
    fn dequeue_group_entity(&mut self, child_group: &Arc<TaskGroup>) {
        let Some(group_attr) = child_group.fair_attr(self.cpu) else {
            return; // root has no group entity attributes
        };
        self.remove_entity_by_id(group_attr.id);
    }
}

impl SchedClassRq for FairClassRq {
    fn enqueue(&mut self, task: Arc<Task>, flags: Option<EnqueueFlags>) {
        let thread = task.as_thread().unwrap();
        let fair_attr = &thread.sched_attr().fair;
        let task_group = thread.task_group();

        // If the task belongs to the same group as this runqueue, enqueue directly.
        if let Some(current_tg) = self.task_group()
            && Arc::ptr_eq(&task_group, &current_tg)
        {
            if let Some(lag) = fair_attr.take_migration_lag() {
                fair_attr.set_vruntime(self.min_vruntime.saturating_add(lag));
            } else {
                let vruntime = match flags {
                    Some(EnqueueFlags::Spawn) => self.min_vruntime + self.vtime_slice(self.len()),
                    _ => self.min_vruntime,
                };
                fair_attr.update_vruntime_at_least(vruntime);
            }
            self.enqueue_entity(fair_attr, FairEntity::Thread(task.clone()));
            self.add_queued_task();
            return;
        }

        let was_empty = {
            let mut leaf_rq = task_group.fair_queue(self.cpu).disable_irq().lock();
            let was_empty = leaf_rq.is_empty();
            leaf_rq.enqueue(task, flags);
            was_empty
        };
        self.update_queued_task_count_upwards(&task_group, Self::add_queued_task);

        if was_empty {
            let Some(self_group) = self.task_group() else {
                return;
            };

            let mut child_group = task_group.clone();
            while let Some(parent_group) = child_group.parent() {
                let parent_was_empty = if Arc::ptr_eq(&parent_group, &self_group) {
                    self.enqueue_group_entity(&child_group)
                } else {
                    parent_group
                        .fair_queue(self.cpu)
                        .disable_irq()
                        .lock()
                        .enqueue_group_entity(&child_group)
                };

                if !parent_was_empty || Arc::ptr_eq(&parent_group, &self_group) {
                    break;
                }
                child_group = parent_group;
            }
        }
    }

    fn len(&self) -> usize {
        self.entities.len()
    }

    fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        loop {
            let item = self.entities.iter().next()?.clone();
            let key = item.key();
            let owner = item.entity().clone();

            match owner {
                FairEntity::Thread(task) => {
                    self.remove_entity(key);
                    self.remove_queued_task();
                    return Some(task);
                }
                FairEntity::Group(child_group) => {
                    let (task, child_is_empty) = {
                        let mut child_rq = child_group.fair_queue(self.cpu).disable_irq().lock();
                        let task = child_rq.pick_next();
                        let child_is_empty = child_rq.is_empty();
                        (task, child_is_empty)
                    };

                    let Some(task) = task else {
                        self.remove_entity(key);
                        continue;
                    };

                    self.remove_queued_task();
                    if child_is_empty {
                        self.remove_entity(key);
                    }

                    return Some(task);
                }
            }
        }
    }

    fn update_current(&mut self, rt: &CurrentRuntime, thread: &Thread, flags: UpdateFlags) -> bool {
        let attr = thread.sched_attr();
        let task_group = thread.task_group();
        let Some(self_group) = self.task_group() else {
            return false;
        };

        let mut should_preempt = if Arc::ptr_eq(&task_group, &self_group) {
            self.update_current_entity(&attr.fair, rt, flags)
        } else {
            let mut leaf_rq = task_group.fair_queue(self.cpu).disable_irq().lock();
            leaf_rq.update_current_entity(&attr.fair, rt, flags)
        };

        let mut current_group = task_group;
        while let Some(parent_group) = current_group.parent() {
            let Some(group_attr) = current_group.fair_attr(self.cpu) else {
                break;
            };

            if Arc::ptr_eq(&parent_group, &self_group) {
                should_preempt |= self.update_current_entity(group_attr, rt, flags);
            } else {
                let mut parent_rq = parent_group.fair_queue(self.cpu).disable_irq().lock();
                should_preempt |= parent_rq.update_current_entity(group_attr, rt, flags);
            }
            current_group = parent_group;
        }

        should_preempt
    }
}
