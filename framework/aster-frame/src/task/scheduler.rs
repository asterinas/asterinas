// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use spin::Once;

use crate::{arch::timer::register_callback, prelude::*, sync::SpinLock, task::Task};

use super::processor::switch_to_task;

pub(crate) static SCHEDULER: Once<&'static dyn Scheduler<Task>> = Once::new();

/*pub struct GlobalScheduler {
    scheduler: &'static dyn Scheduler<Task>,
}

impl GlobalScheduler {
    pub const fn new(scheduler: &'static dyn Scheduler) -> Self {
        Self { scheduler }
    }

    fn enqueue(&mut self, runnable: Arc<Task>, flags: EnqueueFlags) -> bool {
        self.scheduler.enqueue(runnable, flags)
    }
}*/

pub fn inject_scheduler(scheduler: &'static dyn Scheduler) {
    SCHEDULER.call_once(|| { scheduler });
    register_callback(|| {
        SCHEDULER.get().unwrap().local_mut_rq_with(&mut |local_rq| {
            if local_rq.update_current(UpdateFlags::Tick) {
                local_rq.set_should_preempt(true);
            }
        })
    });
}

pub fn add_task(runnable: &Arc<Task>, flags: EnqueueFlags) {
    // println!("add_task");
    let should_reschedule = SCHEDULER.get().unwrap().enqueue(runnable.clone(), flags);
    // println!("----------------------");
    if !should_reschedule {
        return;
    }
    // println!("should reschedule");
    if flags == EnqueueFlags::Spawn {
        reschedule(&mut |local_rq| {
            if let Some(next_current) = local_rq.pick_next_current() {
                ReschedAction::SwitchTo(next_current.clone())
            } else {
                ReschedAction::DoNothing
            }
        });
    }
}

pub fn reschedule<F>(f: &mut F) 
where
    F: FnMut(&mut dyn LocalRunQueue) -> ReschedAction
{
    let mut next = None;
    let mut flag = true;
    while flag {
        SCHEDULER.get().unwrap().local_mut_rq_with(&mut |local_rq| {
            match f(local_rq) {
                ReschedAction::DoNothing => { flag = false; },
                ReschedAction::Retry => {},
                ReschedAction::SwitchTo(next_task) => {
                    flag = false;
                    next = Some(next_task);
                }
            };
        });
    }
    if let Some(next_task) = next {
        switch_to_task(next_task);
    }
}

pub enum ReschedAction {
    DoNothing,
    Retry,
    SwitchTo(Arc<Task>),
}

/// Abstracts a task scheduler.
pub trait Scheduler<T = Task>: Sync + Send {
    /// Enqueue a runnable task.
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> bool;

    /// Get an immutable access to the local runqueue of the current CPU core.
    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>));

    /// Get a mutable access to the local runqueue of the current CPU core.
    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>));
}

/// The _remote_ view of a per-CPU runqueue.
///
/// This remote view provides the interface for the runqueue of a CPU core
/// to be inspected by the code running on an another CPU core.
pub trait RunQueue: Sync + Send {
    /// Returns whether there are any runnable tasks managed by the scheduler.
    fn is_empty(&self) -> bool;

    /// Returns whether the number of runnable tasks managed by the scheduler.
    fn len(&self) -> usize;
}

/// The _local_ view of a per-CPU runqueue.
///
/// This local view provides the interface for the runqueue of a CPU core
/// to be inspected and manipulated by the code running on this particular CPU core.
///
/// Conceptually, a local runqueue consists of two parts:
/// (1) a priority queue of runnable tasks;
/// (2) the current running task.
/// The exact definition of "priority" is left for the concrete implementation to decide.
pub trait LocalRunQueue<T = Task>: RunQueue {
    /// Update the current runnable task's time statistics and 
    /// potentially its position in the queue.
    fn update_current(&mut self, flags: UpdateFlags) -> bool;

    /// Dequeue the current runnable task.
    ///
    /// The current task should be dequeued if it needs to go to sleep or has exited.
    fn dequeue_current(&mut self) -> Option<Arc<T>>;

    /// Pick the next current runnable task, returning the new currenet.
    ///
    /// If there is no runnable task, the method returns `None`.
    fn pick_next_current(&mut self) -> Option<&Arc<T>>;

    /// Gets the current task.
    ///
    /// The current task is the head of the queue.
    /// If the queue is empty, the method returns `None`.
    fn current(&self) -> Option<&Arc<T>>;

    fn set_should_preempt(&mut self, should_preempt: bool);

    fn should_preempt(&self) -> bool;
}

pub enum QueueReceipt {}

#[derive(PartialEq, Copy, Clone)]
pub enum EnqueueFlags {
    Spawn,
    Wake,
}

#[derive(PartialEq, Copy, Clone)]
pub enum UpdateFlags {
    Tick,
    Wait,
    Yield,
}

pub struct CpuId;