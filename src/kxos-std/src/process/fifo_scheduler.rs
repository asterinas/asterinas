use alloc::{boxed::Box, collections::VecDeque, sync::Arc};
use kxos_frame::task::{set_scheduler, Scheduler, Task};
use kxos_frame::sync::SpinLock;

pub const TASK_INIT_CAPABILITY: usize = 16;

pub struct FifoScheduler {
    tasks: SpinLock<VecDeque<Arc<Task>>>,
}

impl FifoScheduler {
    pub fn new() -> Self {
        Self {
            tasks: SpinLock::new(VecDeque::with_capacity(TASK_INIT_CAPABILITY)),
        }
    }
}

impl Scheduler for FifoScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        self.tasks.lock().push_back(task.clone());
    }

    fn dequeue(&self) -> Option<Arc<Task>> {
        self.tasks.lock().pop_front()
    }
}

pub fn init() {
    let fifo_scheduler = Box::new(FifoScheduler::new());
    let scheduler = Box::<FifoScheduler>::leak(fifo_scheduler);
    set_scheduler(scheduler);
}
