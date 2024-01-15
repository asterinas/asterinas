use super::{
    prio_arr::PriorityArray,
    sched_entity::{PriorityOp, SchedEntity, STARVATION_LIMIT},
};
use alloc::sync::Arc;
use aster_frame::{
    task::{NeedResched, Priority, ReadPriority, WakeUp},
    timer::current_tick,
};

/// Queues of runnable tasks.
pub struct RunQueue<T: NeedResched + ReadPriority + WakeUp> {
    /// The runnable processes have not yet exhausted their time slice
    /// and are thus allowed to run.
    active: PriorityArray<T>,
    /// The runnable processes have exhausted their time quantum
    /// and are thus forbidden to run until all active processes expire.
    expired: PriorityArray<T>,

    /// The highest priority of tasks in the expired array.
    best_expired_prio: Option<Priority>,
    /// The ticks of the first task added into the expired arrays.
    /// For stavation detection.
    first_expired_timestamp: u64,
    /// The latest picked task from this runqueue.
    latest_picked: Option<Arc<SchedEntity<T>>>,
}

impl<T: NeedResched + ReadPriority + WakeUp> Default for RunQueue<T> {
    fn default() -> Self {
        Self {
            active: PriorityArray::<T>::default(),
            expired: PriorityArray::<T>::default(),
            best_expired_prio: None,
            first_expired_timestamp: 0,
            latest_picked: None,
        }
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> RunQueue<T> {
    /// The total number of tasks in the runqueue.
    #[inline]
    pub fn total_num(&self) -> aster_frame::task::TaskNumber {
        self.active.total_num() + self.expired.total_num()
    }

    fn swap_to_refill_active(&mut self) {
        debug_assert!(self.active.empty());
        core::mem::swap(&mut self.active, &mut self.expired);
        self.first_expired_timestamp = 0;
    }

    pub fn activate(&mut self, entity: Arc<SchedEntity<T>>) {
        debug_assert!(entity.time_slice_in_tick() > 0);
        entity.set_on_queue(true);
        entity.set_need_resched_to(false);
        let cur_tick = current_tick();
        entity.update(|inner| {
            inner.timestamp_tick = Some(cur_tick);
        });
        self.active.enqueue(entity);
    }

    /// pick the next task to run from the active queues
    pub fn pick_next(&mut self) -> Option<Arc<SchedEntity<T>>> {
        if self.active.empty() && !self.expired.empty() {
            self.swap_to_refill_active();
        }

        self.latest_picked = self.active.pick_next().map(|next_entity| {
            next_entity.update(|inner| inner.timestamp_tick = Some(current_tick()));
            next_entity
        });

        if let Some(picked) = &self.latest_picked {
            picked.set_on_queue(false);
        }

        self.latest_picked.clone()
    }

    /// Return `true` if the sched_entity is found in the runqueue.
    pub fn remove(&mut self, entity: &Arc<SchedEntity<T>>) -> bool {
        let cur_tick = current_tick();
        entity.update(|inner| {
            inner.timestamp_tick = Some(cur_tick);
        });

        self.active.dequeue(entity) || self.expired.dequeue(entity)
    }

    /// Interactive tasks are attempted to be placed back into the
    /// active array for better responsiveness.
    /// This method provides a load-dependent heuristic to prevent
    /// tasks in the expired array from starving.
    pub fn expired_is_starving(&self) -> bool {
        self.latest_picked
            .as_ref()
            .map(|picked| {
                self.best_expired_prio
                    // ignore the interactivity if a task with a better static priority has expired
                    .is_some_and(|best_expired| best_expired >= picked.prio())
            })
            .unwrap_or(false)

            // whether the *first* expired task had to wait more than a 'reasonable' amount of time.
            // The deadline timeout depends on the number of running tasks.
            || (STARVATION_LIMIT != 0
                && self.first_expired_timestamp != 0
                && (current_tick() - self.first_expired_timestamp) as u64
                    > STARVATION_LIMIT
                        * (self.active.total_num() + self.expired.total_num()) as u64)
    }

    pub fn expire(&mut self, entity: Arc<SchedEntity<T>>) {
        entity.set_on_queue(true);
        let cur_tick = current_tick();
        self.expire_with_tick(entity, cur_tick);
        if self.first_expired_timestamp == 0 {
            self.first_expired_timestamp = cur_tick;
        }
    }

    fn expire_with_tick(&mut self, entity: Arc<SchedEntity<T>>, cur_tick: u64) {
        entity.set_need_resched_to(false);
        entity.update(|inner| {
            inner.timestamp_tick = Some(cur_tick);
        });
        self.expired.enqueue(entity);
    }

    /// For better interactive performance,
    /// requeue the eq-priority tasks within the active array.
    /// Prevent a too long timeslice allowing a task to monopolize
    /// the CPU by splitting up the timeslice into smaller pieces,
    /// so that the task can be preempted by other eq-priority tasks.
    ///
    /// # Return
    ///
    /// `true` if the task need to be requeued, `false` otherwise.
    pub fn try_mark_to_requeue(&mut self, entity: Arc<SchedEntity<T>>) -> bool {
        if self.active.empty() || self.active.is_empty_in(&entity.dyn_prio()) {
            return false;
        }
        todo!("add interactive detection and requeue judgement");
        // false
    }

    pub fn has_active_task_with_higher_prio_than(&self, prio: &Priority) -> bool {
        self.active
            .highest_prio()
            .is_some_and(|highest_prio| highest_prio > *prio)
    }

    #[inline]
    pub fn update_best_expired_prio_with(&mut self, prio: Priority) {
        self.best_expired_prio = Some(prio.max(self.best_expired_prio.unwrap_or(prio)));
    }

    pub fn contains(&self, entity: &Arc<SchedEntity<T>>) -> bool {
        self.active.contains(entity) || self.expired.contains(entity)
    }
}
