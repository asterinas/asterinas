use crate::prelude::*;
use aster_frame::config::REAL_TIME_TASK_PRI;
use aster_frame::task::{NeedResched, Priority, ReadPriority, Task, WakeUp};
use aster_frame::timer::current_tick;

type EntityIdx = usize;

pub(super) struct SchedEntity<T: NeedResched + ReadPriority + WakeUp = Task> {
    task: Arc<T>,
    inner: SpinLock<SchedEntityInner>,
}

type Tick = u64;

#[derive(Debug, PartialEq, Eq)]
pub struct SchedEntityInner {
    /// The dynamic priority of the task,
    /// which is used to determine the task's order in the runqueue.
    pub dyn_prio: Priority,
    pub avg_sleep_tick: Tick,

    /// The time slice of the task in ticks.
    pub time_slice_in_tick: Tick,
    pub full_time_slice_in_tick: Option<Tick>,

    pub timestamp_tick: Option<Tick>,

    /// Whether the entity has been on the runqueue,
    /// or will be placed in the runqueue *in a very short time*.
    pub on_queue: bool,

    /// Skip the next dynamic priority update.
    /// Should be reset to `false` after the skip.
    skip_dyn_prio_update: bool,
}

impl Default for SchedEntityInner {
    fn default() -> Self {
        Self {
            dyn_prio: Priority::normal(),
            avg_sleep_tick: 0,
            time_slice_in_tick: 0,
            timestamp_tick: None,
            full_time_slice_in_tick: None,
            skip_dyn_prio_update: false,
            on_queue: false,
        }
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> SchedEntity<T> {
    fn new(task: Arc<T>) -> Self {
        let inner = SchedEntityInner {
            dyn_prio: task.priority(),
            ..Default::default()
        };
        Self {
            task,
            inner: SpinLock::new(inner),
        }
    }

    #[cfg(any(test, ktest))]
    pub(super) fn new_mock(task: Arc<T>) -> Self {
        Self::new(task)
    }

    pub fn index(&self) -> EntityIdx {
        entity_table::to_table_idx(&self.task)
    }

    pub fn update<R: Send + Sync>(
        &self,
        modifier: impl FnOnce(&mut SpinLockGuard<'_, SchedEntityInner>) -> R,
    ) -> R {
        let mut inner = self.inner.lock();
        modifier(&mut inner)
    }

    pub fn full_time_slice_in_tick(&self) -> Option<Tick> {
        self.inner.lock().full_time_slice_in_tick
    }

    pub fn time_slice_in_tick(&self) -> Tick {
        self.inner.lock().time_slice_in_tick
    }

    pub fn timestamp_tick(&self) -> Option<Tick> {
        self.inner.lock().timestamp_tick
    }

    /// Set the need_resched flag to the given value.
    /// Return the original value.
    pub fn set_need_resched_to(&self, need_resched: bool) -> bool {
        let orig = self.task.need_resched();
        if orig != need_resched {
            if need_resched {
                self.task.set_need_resched();
            } else {
                self.task.clear_need_resched();
            }
        }
        orig
    }

    pub fn need_resched(&self) -> bool {
        self.task.need_resched()
    }

    /// Deny the first time slice flag, and decrease the time slice count.
    ///
    /// # Return
    ///
    /// The remaining time slice (in tick number).
    pub fn tick(&self) -> Tick {
        self.update(|inner| {
            inner.time_slice_in_tick = inner.time_slice_in_tick.saturating_sub(1);
            inner.time_slice_in_tick
        })
    }

    pub fn update_sleep_avg(&self, cumulative: bool) {
        let cur_tick = current_tick();
        let inner = &mut self.inner.lock();
        let passed_ticks = cur_tick - inner.timestamp_tick.unwrap();
        inner.avg_sleep_tick = if cumulative {
            MAX_SLEEP_AVG.min(passed_ticks + inner.avg_sleep_tick)
        } else {
            inner.avg_sleep_tick.saturating_sub(passed_ticks)
        };
    }

    pub fn task(&self) -> Arc<T> {
        self.task.clone()
    }

    pub fn on_queue(&self) -> bool {
        self.inner.lock().on_queue
    }

    pub fn set_on_queue(&self, on_queue: bool) {
        let inner = &mut self.inner.lock();
        if inner.on_queue == on_queue {
            panic!("on_queue should not be set to the same value: {}", on_queue);
        }
        inner.on_queue = on_queue;
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> PartialEq for SchedEntity<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}
impl<T: NeedResched + ReadPriority + WakeUp> Eq for SchedEntity<T> {}

pub trait PriorityOp {
    /// The static priority from the inner task.
    fn prio(&self) -> Priority;

    /// The dynamic priority.
    fn dyn_prio(&self) -> Priority;

    /// real-time originally, or boosted to real-time
    fn seen_as_real_time(&self) -> bool;

    /// Update the dynamic priority.
    /// For real-time tasks, the dynamic priority will not be updated.
    /// For normal tasks, the dynamic priority will be updated according
    /// to the current bonus which depends on the sleep average.
    ///
    /// Return the updated dynamic priority.
    fn update_dyn_prio(&self) -> Priority;

    /// Mark the next dynamic priority update as skipped.
    fn skip_next_dyn_prio_update(&self);
}

impl<T: NeedResched + ReadPriority + WakeUp> PriorityOp for SchedEntity<T> {
    fn prio(&self) -> Priority {
        self.task.priority()
    }
    fn dyn_prio(&self) -> Priority {
        self.inner.lock().dyn_prio
    }
    fn seen_as_real_time(&self) -> bool {
        self.task.is_real_time() || self.boosted_to_real_time()
    }

    fn update_dyn_prio(&self) -> Priority {
        let mut inner = self.inner.lock();
        if inner.skip_dyn_prio_update {
            inner.skip_dyn_prio_update = false;
            return inner.dyn_prio;
        }

        inner.dyn_prio = Priority::new({
            // The bonus is a value ranging from 0 to 10.
            // A value less than 5 represents a penalty that lowers the dynamic priority,
            // while a value greater than 5 is a premium that raises the dynamic priority.
            let current_bouns =
                (inner.avg_sleep_tick * bonus::MAX_BONUS as u64 / MAX_SLEEP_AVG) as u16;
            let prio = self.task.priority().get() - current_bouns + bonus::MAX_BONUS / 2;
            prio.max(REAL_TIME_TASK_PRI).min(Priority::lowest().get())
        });
        inner.dyn_prio
    }
    fn skip_next_dyn_prio_update(&self) {
        self.inner.lock().skip_dyn_prio_update = true;
    }
}

pub trait WokenUpTimestamp {
    fn woken_up_timestamp(&self) -> Option<Tick>;
    fn clear_woken_up_timestamp(&self);
}

impl<T: NeedResched + ReadPriority + WakeUp> WokenUpTimestamp for SchedEntity<T> {
    fn woken_up_timestamp(&self) -> Option<Tick> {
        self.task.woken_up_timestamp()
    }
    fn clear_woken_up_timestamp(&self) {
        self.task.clear_woken_up_timestamp()
    }
}

pub mod entity_table {
    use super::*;
    use hashbrown::HashMap;

    pub type EntityTable<T = Task> = SpinLock<HashMap<EntityIdx, Arc<SchedEntity<T>>>>;

    pub fn new_entity_table<T: NeedResched + ReadPriority + WakeUp>() -> EntityTable<T> {
        SpinLock::new(HashMap::new())
    }

    pub(in crate::sched::scheduler::multiqueue) trait EntityTableOp<
        T: NeedResched + ReadPriority + WakeUp,
    >
    {
        fn has_entity(&self, task: &Arc<T>) -> bool;
        fn to_entity(&self, task: &Arc<T>) -> Arc<SchedEntity<T>>;
        fn drop_entity(&self, task: &Arc<T>) -> Option<Arc<SchedEntity<T>>>;
    }

    pub(super) fn to_table_idx<T>(task: &Arc<T>) -> EntityIdx {
        Arc::as_ptr(task) as EntityIdx
    }

    /// Get the sched entity from the table with the given task.
    /// Allocate a new sched entity if the corresponding one does not exist.
    /// The same task is mapped to the same sched entity.
    pub fn to_entity_in_table<T: NeedResched + ReadPriority + WakeUp>(
        table: &EntityTable<T>,
        task: &Arc<T>,
    ) -> Arc<SchedEntity<T>> {
        table
            .lock()
            .entry(to_table_idx(task))
            .or_insert_with(|| Arc::new(SchedEntity::new(task.clone())))
            .clone()
    }

    /// Remove the sched entity from the table.
    /// Return the removed sched entity in a `Some` if it exists, or `None` otherwise.
    pub fn drop_entity_from_table<T: NeedResched + ReadPriority + WakeUp>(
        table: &EntityTable<T>,
        task: &Arc<T>,
    ) -> Option<Arc<SchedEntity<T>>> {
        table.lock().remove(&to_table_idx(task))
    }

    /// Whether the corresponding sched entity exists in the table.
    pub fn has_entity_in_table<T: NeedResched + ReadPriority + WakeUp>(
        table: &EntityTable<T>,
        task: &Arc<T>,
    ) -> bool {
        table.lock().contains_key(&to_table_idx(task))
    }
}

mod bonus {
    const BONUS_RATIO: u16 = 25;
    const MAX_USER_PRIO: u16 = 40;
    pub const MAX_BONUS: u16 = MAX_USER_PRIO * BONUS_RATIO / 100;
}

const MAX_SLEEP_AVG: Tick = super::timeslice::DEFAULT_TIME_SLICE * bonus::MAX_BONUS as Tick;
pub(super) const STARVATION_LIMIT: Tick = MAX_SLEEP_AVG;

pub trait Responsiveness {
    const INTERACTIVE_DELTA: i32;

    fn is_interactive(&self) -> bool;

    /// Whether the task's dynamic priority is boosted to be real-time.
    /// If the task is already real-time, then also return `true`.
    fn boosted_to_real_time(&self) -> bool;

    fn delta(&self) -> i32;
}

impl<T: NeedResched + ReadPriority + WakeUp> Responsiveness for SchedEntity<T> {
    const INTERACTIVE_DELTA: i32 = 2;

    fn is_interactive(&self) -> bool {
        self.dyn_prio().get() as i32 <= self.task.priority().get() as i32 - self.delta()
    }

    fn delta(&self) -> i32 {
        self.task.nice() as i32 * bonus::MAX_BONUS as i32 / 40 + Self::INTERACTIVE_DELTA
    }

    fn boosted_to_real_time(&self) -> bool {
        self.dyn_prio().is_real_time()
    }
}

#[if_cfg_ktest]
mod tests {
    use super::*;
    use crate::sched::scheduler::multiqueue::test::MockTask;
    type MockSchedEntity = SchedEntity<MockTask>;

    mod entity {
        use super::*;
        #[ktest]
        fn dyn_prio_sync_on_init() {
            let prio = Priority::new(110);
            let task = Arc::new(MockTask::new("test", prio));
            let entity = MockSchedEntity::new(task);
            assert_eq!(entity.dyn_prio(), prio);
        }

        #[ktest]
        fn tick() {
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            let entity = MockSchedEntity::new(task.clone());
            // initial states
            assert_eq!(entity.time_slice_in_tick(), 0);
            // tick on 0 time slice before charged
            assert_eq!(entity.tick(), 0);
            assert_eq!(entity.time_slice_in_tick(), 0);

            // charge
            entity.update(|inner| {
                inner.time_slice_in_tick = 3;
            });
            assert_eq!(entity.time_slice_in_tick(), 3);
            assert_eq!(entity.tick(), 2);
            assert_eq!(entity.tick(), 1);
            assert_eq!(entity.tick(), 0);
            assert_eq!(entity.time_slice_in_tick(), 0);

            // tick on 0 time slice after exhausted
            assert_eq!(entity.tick(), 0);
            assert_eq!(entity.time_slice_in_tick(), 0);
        }

        #[ktest]
        fn woken_up_timestamp() {
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            let entity = MockSchedEntity::new(task.clone());
            assert!(entity.woken_up_timestamp().is_none());

            task.set_woken_up_timestamp(0);
            assert_eq!(entity.woken_up_timestamp(), Some(0));

            entity.clear_woken_up_timestamp();
            assert!(entity.woken_up_timestamp().is_none());
        }

        #[ktest]
        fn skip_next_dyn_prio_update() {
            let init_prio = Priority::new(103);
            let task = Arc::new(MockTask::new("test", init_prio));
            let entity = MockSchedEntity::new(task.clone());

            assert_eq!(entity.dyn_prio(), init_prio);
            entity.skip_next_dyn_prio_update();

            task.set_woken_up_timestamp(0);
            assert_eq!(entity.dyn_prio(), init_prio);
            assert_eq!(entity.update_dyn_prio(), init_prio);

            // the update after the next one(above) should not be skipped
            assert_ne!(entity.update_dyn_prio(), init_prio);
        }
    }

    mod table {
        use self::entity_table::drop_entity_from_table;

        use super::*;
        use entity_table::*;

        #[ktest]
        fn has_entity() {
            let table = new_entity_table();
            let task_1 = Arc::new(MockTask::new("test_1", Priority::normal()));
            let task_2 = Arc::new(MockTask::new("test_2", Priority::normal()));

            assert!(!has_entity_in_table(&table, &task_1) && !has_entity_in_table(&table, &task_2));
            let entity_1 = to_entity_in_table(&table, &task_1);
            assert!(has_entity_in_table(&table, &task_1) && !has_entity_in_table(&table, &task_2));
            let entity_1_cp = entity_1.clone();
            assert!(has_entity_in_table(&table, &entity_1_cp.task()));
            drop_entity_from_table(&table, &entity_1_cp.task());
            assert!(!has_entity_in_table(&table, &task_1));
        }

        #[ktest]
        fn to_entity() {
            let table = new_entity_table();
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            assert!(!has_entity_in_table(&table, &task));

            let entity = to_entity_in_table(&table, &task);
            assert_eq!(entity.task(), task);
            assert!(has_entity_in_table(&table, &task));
            assert_eq!(table.lock().len(), 1);
        }

        #[ktest]
        fn to_entity_unique() {
            let table = new_entity_table();
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            assert!(!has_entity_in_table(&table, &task));

            let entity = to_entity_in_table(&table, &task);
            let the_same_task = task.clone();
            let entity_2 = to_entity_in_table(&table, &the_same_task);
            assert!(has_entity_in_table(&table, &task));
            assert_eq!(table.lock().len(), 1);
            assert_eq!(entity.task(), entity_2.task());
        }

        #[ktest]
        fn drop_entity() {
            let table = new_entity_table();
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            let entity = to_entity_in_table(&table, &task);

            assert!(drop_entity_from_table(&table, &task).is_some_and(|e| e == entity));
            assert!(!has_entity_in_table(&table, &task));
            assert!(drop_entity_from_table(&table, &task).is_none());
        }

        #[ktest]
        fn drop_entity_unique() {
            let table = new_entity_table();
            let task = Arc::new(MockTask::new("test", Priority::normal()));
            let entity = to_entity_in_table(&table, &task);
            entity.update(|inner| {
                inner.dyn_prio = Priority::new(110);
            });

            let the_same_task = task.clone();
            assert!(drop_entity_from_table(&table, &the_same_task).is_some_and(|e| e == entity));
            assert!(!has_entity_in_table(&table, &task));
            assert!(drop_entity_from_table(&table, &task).is_none());
        }
    }
}
