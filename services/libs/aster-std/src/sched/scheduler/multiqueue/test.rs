//! Mock types for testing.

use super::sched_entity::SchedEntity;
use alloc::sync::Arc;
use alloc::{borrow::ToOwned, string::String};
use aster_frame::sync::Mutex;
use aster_frame::task::Scheduler;

use super::sched_entity::entity_table;
use aster_frame::task::{NeedResched, Priority, ReadPriority, WakeUp};

#[derive(Debug)]
pub(super) struct MockTask {
    name: String,
    priority: Priority,
    inner: Mutex<MockTaskInner>,
}

impl MockTask {
    pub fn new(name: &str, priority: Priority) -> Self {
        use crate::alloc::string::ToString;
        Self {
            name: name.to_string(),
            priority,
            inner: Mutex::new(MockTaskInner {
                need_resched: false,
                woken_up_timestamp: None,
            }),
        }
    }
}

pub(super) fn new_mock_sched_entity(name: &str, priority: Priority) -> Arc<SchedEntity<MockTask>> {
    Arc::new(SchedEntity::new_mock(Arc::new(MockTask::new(
        name, priority,
    ))))
}

impl PartialEq for MockTask {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.priority == other.priority && {
            let inner = self.inner.lock();
            let other_inner = other.inner.lock();
            inner.need_resched == other_inner.need_resched
                && inner.woken_up_timestamp == other_inner.woken_up_timestamp
        }
    }
}
impl Eq for MockTask {}

#[derive(Debug, PartialEq, Eq)]
struct MockTaskInner {
    pub need_resched: bool,
    pub woken_up_timestamp: Option<u64>,
}

impl NeedResched for MockTask {
    fn need_resched(&self) -> bool {
        self.inner.lock().need_resched
    }

    fn set_need_resched(&self) {
        self.inner.lock().need_resched = true;
    }

    fn clear_need_resched(&self) {
        self.inner.lock().need_resched = false;
    }
}

impl ReadPriority for MockTask {
    fn priority(&self) -> Priority {
        self.priority
    }

    fn is_real_time(&self) -> bool {
        self.priority.is_real_time()
    }

    fn nice(&self) -> i8 {
        assert!(!self.priority.is_real_time());
        self.priority.as_nice().unwrap()
    }
}

impl WakeUp for MockTask {
    fn woken_up_timestamp(&self) -> Option<u64> {
        self.inner.lock().woken_up_timestamp
    }

    fn clear_woken_up_timestamp(&self) {
        self.inner.lock().woken_up_timestamp = None;
    }

    fn wakeup(&self) {
        unimplemented!(
            "MockTask::wakeup should not be called in tests, use `set_woken_up_timestamp` instead."
        );
    }
}

impl MockTask {
    pub fn set_woken_up_timestamp(&self, timestamp: u64) {
        self.inner.lock().woken_up_timestamp = Some(timestamp);
    }
}

pub(super) struct Worker<Task, SchedulerImpl>
where
    Task: NeedResched + ReadPriority + WakeUp,
    SchedulerImpl: Scheduler<Task> + entity_table::EntityTableOp<Task> + Sync + Send,
{
    pub name: String,
    current_task: Option<Arc<Task>>,
    scheduler: Arc<SchedulerImpl>,
}

impl<Task, SchedulerImpl> Worker<Task, SchedulerImpl>
where
    Task: NeedResched + ReadPriority + WakeUp,
    SchedulerImpl: Scheduler<Task> + entity_table::EntityTableOp<Task> + Sync + Send,
{
    pub fn new(name: &str, scheduler: Arc<SchedulerImpl>) -> Self {
        Self {
            name: name.to_owned(),
            current_task: None,
            scheduler,
        }
    }

    pub fn current_task(&self) -> Option<Arc<Task>> {
        self.current_task.clone()
    }

    pub fn set_current_task(&mut self, task: Option<Arc<Task>>) {
        self.current_task = task;
    }

    pub fn scheduler(&self) -> &Arc<SchedulerImpl> {
        &self.scheduler
    }
}

type SchedImpl = super::MultiQueueScheduler<MockTask>;

mod basic {
    use super::*;

    #[ktest]
    fn add_task() {
        let scheduler = Arc::new(SchedImpl::new());
        let worker = Worker::new("worker", scheduler);

        let task = Arc::new(MockTask::new("task", Priority::normal()));
        assert_eq!(worker.scheduler().task_num(), 0);
        worker.scheduler().enqueue(task.clone());
        assert_eq!(worker.scheduler().task_num(), 1);
        assert_eq!(worker.scheduler().pick_next_task(), Some(task));
    }

    mod remove_task {
        use super::*;

        #[ktest]
        fn single() {
            let scheduler = Arc::new(SchedImpl::new());
            let worker = Worker::new("worker", scheduler);

            let task = Arc::new(MockTask::new("task", Priority::normal()));
            worker.scheduler().enqueue(task.clone());
            assert_eq!(worker.scheduler().task_num(), 1);
            worker.scheduler().remove(&task);
            assert_eq!(worker.scheduler().task_num(), 0);
            assert_eq!(worker.scheduler().pick_next_task(), None);
        }

        #[ktest]
        fn multiple() {
            let scheduler = Arc::new(SchedImpl::new());
            let worker = Worker::new("worker", scheduler);

            let task1 = Arc::new(MockTask::new("task_1", Priority::normal()));
            let task2 = Arc::new(MockTask::new("task_2", Priority::normal()));
            let task3 = Arc::new(MockTask::new("task_3", Priority::normal()));
            worker.scheduler().enqueue(task1.clone());
            worker.scheduler().enqueue(task2.clone());
            worker.scheduler().enqueue(task3.clone());
            assert_eq!(worker.scheduler().task_num(), 3);

            worker.scheduler().remove(&task2);
            assert!(!worker.scheduler().contains(&task2));
            assert_eq!(worker.scheduler().task_num(), 2);
        }
    }

    mod fetch_next_task {
        use super::*;
        // the case that only 1 task in the queue has been tested in `add_task`.

        #[ktest]
        fn higher_prio_first() {
            let scheduler = Arc::new(SchedImpl::new());
            let worker = Worker::new("worker", scheduler);

            let task_1 = Arc::new(MockTask::new("task_1", Priority::new(20)));
            let task_2 = Arc::new(MockTask::new("task_2", Priority::new(102)));
            assert!(task_1.priority() > task_2.priority());

            worker.scheduler().enqueue(task_2.clone());
            worker.scheduler().enqueue(task_1.clone());
            assert_eq!(worker.scheduler().task_num(), 2);
            assert_eq!(worker.scheduler().pick_next_task(), Some(task_1));
            assert_eq!(worker.scheduler().pick_next_task(), Some(task_2));
        }

        #[ktest]
        fn eq_prio_fifo() {
            let scheduler = Arc::new(SchedImpl::new());
            let worker = Worker::new("worker", scheduler);

            let task_1 = Arc::new(MockTask::new("task_1", Priority::new(100)));
            let task_2 = Arc::new(MockTask::new("task_2", Priority::new(100)));

            worker.scheduler().enqueue(task_1.clone());
            worker.scheduler().enqueue(task_2.clone());
            assert_eq!(worker.scheduler().task_num(), 2);
            assert_eq!(worker.scheduler().pick_next_task(), Some(task_1));
            assert_eq!(worker.scheduler().pick_next_task(), Some(task_2));
        }
    }
}
