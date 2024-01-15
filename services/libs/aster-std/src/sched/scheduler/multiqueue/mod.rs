//! O1 scheduler implementation.

mod prio_arr;
mod runqueue;
mod sched_entity;
#[cfg(any(test, ktest))]
mod test;
mod timeslice;

use crate::prelude::*;
use aster_frame::config::SCHED_DEBUG_LOG;
use aster_frame::task::{NeedResched, Priority, ReadPriority, Scheduler, Task, WakeUp};
use sched_entity::entity_table::{EntityTable, EntityTableOp};
use sched_entity::{PriorityOp, Responsiveness, SchedEntity, WokenUpTimestamp};

use self::sched_entity::entity_table;

type QueueLock<T> = SpinLock<T>;

pub struct MultiQueueScheduler<T: NeedResched + ReadPriority + WakeUp = Task> {
    rq: QueueLock<runqueue::RunQueue<T>>,
    /// A map from task address to its scheduling entity
    entity_table: EntityTable<T>,
}

impl<T: NeedResched + ReadPriority + WakeUp> MultiQueueScheduler<T> {
    pub fn new() -> Self {
        Self {
            rq: QueueLock::new(runqueue::RunQueue::<T>::default()),
            entity_table: entity_table::new_entity_table(),
        }
    }

    /// Calculate the time slice for a task.
    /// Same way for both RR real-time and normal tasks.
    fn calc_time_slice(&self, entity: &Arc<SchedEntity<T>>) -> u64 {
        timeslice::TimeSlice::from(entity.prio()).as_ticks()
    }

    /// Charge a task's time slice.
    fn charge(&self, entity: &Arc<SchedEntity<T>>) {
        if !entity.seen_as_real_time() {
            entity.update_dyn_prio();
        }

        entity.update(|inner| {
            inner.time_slice_in_tick = *inner
                .full_time_slice_in_tick
                .get_or_insert_with(|| self.calc_time_slice(entity));
        });
    }

    fn empty(&self) -> bool {
        self.rq.lock().total_num() == 0
    }

    fn enqueue_entity(&self, entity: Arc<SchedEntity<T>>) {
        if entity.on_queue() {
            // to prevent the problem(mentioned in #565) that
            // an on-queue task can be enqueue multiple times.
            return;
        }

        let newly_created = entity.timestamp_tick().is_none();
        if newly_created {
            self.charge(&entity);
            self.rq.lock().activate(entity);
        } else {
            if let Some(woken_up_timestamp) = entity.woken_up_timestamp() {
                entity.update(|inner| inner.timestamp_tick = Some(woken_up_timestamp));
                entity.clear_woken_up_timestamp();
                entity.update_sleep_avg(true);
            } else {
                entity.update_sleep_avg(false);
            }

            if entity.time_slice_in_tick() == 0 {
                self.charge(&entity);
                let rq = &mut self.rq.lock();
                if entity.is_interactive() && !rq.expired_is_starving() {
                    rq.update_best_expired_prio_with(entity.prio());
                    rq.activate(entity);
                } else {
                    rq.expire(entity);
                }
            } else {
                self.rq.lock().activate(entity);
            }
        }
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> EntityTableOp<T> for MultiQueueScheduler<T> {
    fn has_entity(&self, task: &Arc<T>) -> bool {
        sched_entity::entity_table::has_entity_in_table(&self.entity_table, task)
    }

    fn to_entity(&self, task: &Arc<T>) -> Arc<SchedEntity<T>> {
        sched_entity::entity_table::to_entity_in_table(&self.entity_table, task)
    }

    fn drop_entity(&self, task: &Arc<T>) -> Option<Arc<SchedEntity<T>>> {
        sched_entity::entity_table::drop_entity_from_table(&self.entity_table, task)
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> Scheduler<T> for MultiQueueScheduler<T>
where
    MultiQueueScheduler<T>: EntityTableOp<T> + Sync + Send,
{
    /// Move a task to the runqueue and do priority recalculation.
    /// Update all the scheduling statistics stuff.
    /// (sleep average calculation, priority modifiers, etc.)
    fn enqueue(&self, task: Arc<T>) {
        self.enqueue_entity(self.to_entity(&task));
    }

    fn remove(&self, task: &Arc<T>) -> bool {
        if !self.has_entity(task) {
            return false;
        }

        let entity = self.to_entity(task);
        let found = self.rq.lock().remove(&entity);
        self.drop_entity(task);
        found
    }

    fn pick_next_task(&self) -> Option<Arc<T>> {
        self.rq
            .lock()
            .pick_next()
            .as_ref()
            .map(|entity| entity.task())
    }

    fn should_preempt(&self, task: &Arc<T>) -> bool {
        if task.need_resched() && !self.empty() {
            return true;
        }
        let prio = self.to_entity(task).dyn_prio();
        self.rq.lock().has_active_task_with_higher_prio_than(&prio)
    }

    fn tick(&self, task: &Arc<T>) {
        if task.need_resched() {
            return;
        }
        let entity = self.to_entity(task);
        debug_assert!(entity.on_queue());

        if entity.tick() == 0 {
            entity.set_need_resched_to(true);
            if SCHED_DEBUG_LOG {
                debug!("task expired");
            }
        }
    }

    fn before_yield(&self, cur_task: &Arc<T>) {
        let entity = self.to_entity(cur_task);
        if !entity.seen_as_real_time() {
            entity.update(|inner| {
                // move the task into the expired queue
                inner.time_slice_in_tick = 0;
            });
        }
        entity.set_need_resched_to(true);
        if SCHED_DEBUG_LOG {
            debug!("task yielded");
        }
    }

    fn yield_to(&self, cur_task: &Arc<T>, target_task: Arc<T>) {
        if self.has_entity(&target_task) {
            // prevent the target from being enqueued multiple times
            let entity = self.to_entity(&target_task);
            if entity.on_queue() {
                self.rq.lock().remove(&entity);
            }
        };
        let target_entity = self.to_entity(&target_task);
        let target_dyn_prio_val = target_entity.dyn_prio().get();
        self.enqueue_entity(target_entity); // to update the target's dyn_prio

        let cur_entity = self.to_entity(cur_task);
        let new_prio = {
            // make sure that the current's new priority is beneath the target task's.
            let val = Priority::lowest().get().min(target_dyn_prio_val + 1);
            Priority::new(val)
        };
        if cur_entity.dyn_prio() > new_prio {
            cur_entity.update(|inner| {
                inner.dyn_prio = new_prio;
            });
        }
        cur_entity.skip_next_dyn_prio_update();
        cur_entity.set_need_resched_to(true);
    }

    #[cfg(any(test, ktest))]
    fn task_num(&self) -> aster_frame::task::TaskNumber {
        self.rq.lock().total_num()
    }

    #[cfg(any(test, ktest))]
    fn contains(&self, task: &Arc<T>) -> bool {
        if !self.has_entity(task) {
            return false;
        }

        let entity = self.to_entity(task);
        if entity.on_queue() {
            return true;
        }
        self.rq.lock().contains(&entity)
    }
}
