use crate::prelude::*;
use jinux_frame::task::{set_scheduler, Scheduler, Task, TaskAdapter};

use intrusive_collections::LinkedList;

pub struct FifoScheduler {
    tasks: Mutex<LinkedList<TaskAdapter>>,
}

impl FifoScheduler {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(LinkedList::new(TaskAdapter::new())),
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
