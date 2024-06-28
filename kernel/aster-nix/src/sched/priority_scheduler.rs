// SPDX-License-Identifier: MPL-2.0

use aster_frame::{arch::console::print, task::{inject_scheduler, CpuId, EnqueueFlags, LocalRunQueue, RunQueue, Scheduler, Task, TaskAdapter, UpdateFlags}};
use intrusive_collections::{LinkedList, LinkedListLink};

use crate::prelude::*;

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::new());
    let scheduler = Box::<PreemptScheduler>::leak(preempt_scheduler);
    inject_scheduler(scheduler);
}

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler<T = Task> {
    run_queue: SpinLock<PreemptRunQueue<T>>,
}

impl PreemptScheduler {
    pub fn new() -> Self {
        Self { run_queue: SpinLock::new(PreemptRunQueue::new()) }
    }
}

impl Scheduler for PreemptScheduler {
    fn enqueue(&self, runnable: Arc<Task>, flags: EnqueueFlags) -> bool {
        if runnable.is_real_time() {
            // println!("real time enqueued");
            self.run_queue.lock_irq_disabled().real_time_tasks.push_back(runnable);
        } else {
            // println!("normal enqueued");
            self.run_queue.lock_irq_disabled().normal_tasks.push_back(runnable);
        }
        self.run_queue.lock_irq_disabled().len += 1;
        // println!("after enqueue");
        true
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<Task>)) {
        let local_rq: &PreemptRunQueue = &self.run_queue.lock_irq_disabled();
        f(local_rq);
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<Task>)) {
        let local_rq: &mut PreemptRunQueue = &mut self.run_queue.lock_irq_disabled();
        f(local_rq);
    }
}

struct PreemptRunQueue<T = Task> {
    len: usize,
    current: Option<Arc<T>>,
    is_real_time: bool,
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: VecDeque<Arc<T>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: VecDeque<Arc<T>>,
    should_preempt: bool,
}

impl PreemptRunQueue {
    pub fn new() -> Self {
        Self {
            len: 0,
            current: None,
            is_real_time: false,
            real_time_tasks: VecDeque::new(),
            normal_tasks: VecDeque::new(),
            should_preempt: false,
        }
    }
}

impl<T: Sync + Send> RunQueue for PreemptRunQueue<T> {
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn len(&self) -> usize {
        self.len
    }
}

impl<T: Sync + Send> LocalRunQueue<T> for PreemptRunQueue<T> {
    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        flags != UpdateFlags::Tick
    }

    fn dequeue_current(&mut self) -> Option<Arc<T>> {
        let current = self.current.clone();
        self.current = None;
        self.len -= 1;
        current
    }

    fn pick_next_current(&mut self) -> Option<&Arc<T>> {
        let prev = self.current.clone();
        let is_real_time = self.is_real_time;
        self.current = if !self.real_time_tasks.is_empty() {
            self.is_real_time = true;
            self.real_time_tasks.pop_front()
        } else {
            self.is_real_time = false;
            self.normal_tasks.pop_front()
        };
        if self.current.is_none() {
            self.current = prev;
            self.is_real_time = is_real_time;
            return None;
        } else if let Some(prev_task) = prev {
            if is_real_time {
                self.real_time_tasks.push_back(prev_task);
            } else {
                self.normal_tasks.push_back(prev_task);
            }
        }

        self.current.as_ref()
    }

    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref()
    }

    fn set_should_preempt(&mut self, should_preempt: bool) {
        self.should_preempt = should_preempt
    }
    
    fn should_preempt(&self) -> bool {
        self.should_preempt
    }
}
