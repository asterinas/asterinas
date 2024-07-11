// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

mod fifo_scheduler;

pub use fifo_scheduler::FifoScheduler;
use spin::Once;

use super::{processor::switch_to_task, task::Task, TaskStatus};
use crate::{arch::timer::register_callback, prelude::*};

static SCHEDULER: Once<&'static dyn Scheduler<Task>> = Once::new();

/// Injects a scheduler implementation into framework.
///
/// This function can only be called once and must be called during the initialization of kernel.
pub fn inject_scheduler(scheduler: &'static dyn Scheduler) {
    SCHEDULER.call_once(|| scheduler);

    register_callback(|| {
        SCHEDULER.get().unwrap().local_mut_rq_with(&mut |local_rq| {
            if local_rq.update_current(UpdateFlags::Tick) {
                local_rq.set_should_preempt(true);
            }
        })
    });
}

/// Adds task to scheduler.
///
/// This function may be called with newly spawned task or waked task.
pub fn add_task(runnable: Arc<Task>, flags: EnqueueFlags) {
    if !SCHEDULER.is_completed() {
        let fifo_scheduler = Box::new(FifoScheduler::<Task>::new());
        let static_scheduler = Box::leak(fifo_scheduler);
        inject_scheduler(static_scheduler);
    }
    SCHEDULER.get().unwrap().enqueue(runnable, flags);
}

/// Yields execution.
pub fn yield_now(flags: YieldFlags) {
    reschedule(&mut |local_rq| {
        if flags != YieldFlags::Exit {
            local_rq.update_current(flags.into());
        }

        let mut current_task_option = None;
        if flags != YieldFlags::Yield {
            current_task_option = local_rq.dequeue_current();
        }
        let current_task;
        let mut current_task_inner = None;
        if flags == YieldFlags::Wait {
            current_task = current_task_option.clone().unwrap();
            current_task_inner = Some(current_task.inner_exclusive_access());

            if current_task_inner.as_ref().unwrap().task_status == TaskStatus::Runnable {
                drop(current_task_inner);
                local_rq.set_current(current_task_option);
                return ReschedAction::DoNothing;
            }
        }

        if let Some(next) = local_rq.pick_next_current() {
            if flags == YieldFlags::Wait {
                debug_assert_eq!(
                    current_task_inner.as_ref().unwrap().task_status,
                    TaskStatus::Sleepy
                );
                current_task_inner.unwrap().task_status = TaskStatus::Sleeping;
            }
            local_rq.set_current(Some(next.clone()));
            ReschedAction::SwitchTo(next)
        } else {
            match flags {
                YieldFlags::Exit => ReschedAction::Retry,
                YieldFlags::Yield => ReschedAction::DoNothing,
                YieldFlags::Wait => {
                    drop(current_task_inner);
                    local_rq.set_current(current_task_option);
                    ReschedAction::DoNothing
                }
            }
        }
    })
}

/// Invokes a schedule.
pub fn schedule() {
    reschedule(&mut |local_rq| {
        if !local_rq.should_preempt() {
            return ReschedAction::DoNothing;
        }
        local_rq.set_should_preempt(false);

        if let Some(next_current) = local_rq.pick_next_current() {
            local_rq.set_current(Some(next_current.clone()));
            ReschedAction::SwitchTo(next_current)
        } else {
            ReschedAction::DoNothing
        }
    });
}

/// Performs rescheduling.
///
/// This function accepts a closure whose result indicates its possible action.
fn reschedule<F>(f: &mut F)
where
    F: FnMut(&mut dyn LocalRunQueue) -> ReschedAction,
{
    let mut next = None;
    let mut flag = true;
    while flag {
        SCHEDULER.get().unwrap().local_mut_rq_with(&mut |local_rq| {
            match f(local_rq) {
                ReschedAction::DoNothing => {
                    flag = false;
                }
                ReschedAction::Retry => {}
                ReschedAction::SwitchTo(next_task) => {
                    next = Some(next_task);
                    flag = false;
                }
            };
        });
    }
    if let Some(next_task) = next {
        switch_to_task(next_task);
    }
}

/// A per-CPU task scheduler.
///
/// Note: Scheduler developers are responsible for thread-safety of their own schedulers in SMP context.
pub trait Scheduler<T = Task>: Sync + Send {
    /// Enqueue a runnable task.
    ///
    /// Scheduler developers can perform load-balancing or some accounting work here.
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags);

    /// Get an immutable access to the local runqueue of the current CPU core.
    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>));

    /// Get a mutable access to the local runqueue of the current CPU core.
    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>));
}

/// The _local_ view of a per-CPU runqueue.
///
/// This local view provides the interface for the runqueue of a CPU core
/// to be inspected and manipulated by the code running on this particular CPU core.
///
/// Conceptually, a local runqueue consists of two parts:
/// (1) a priority queue of runnable tasks;
/// (2) the current running task.
/// (3) a flag indicating whether current task should be preempted.
/// The exact definition of "priority" is left for the concrete implementation to decide.
pub trait LocalRunQueue<T = Task> {
    /// Updates the current runnable task's time statistics and
    /// potentially its position in the queue.
    ///
    /// If a rescheduling is needed, the method returns true.
    fn update_current(&mut self, flags: UpdateFlags) -> bool;

    /// Removes the current task from the runqueue;
    ///
    /// If there is no current task, the method returns `None`.
    fn dequeue_current(&mut self) -> Option<Arc<T>>;

    /// Picks the next current runnable task, returning the chosen candidate.
    ///
    /// If there is no runnable task, the method returns `None`.
    fn pick_next_current(&mut self) -> Option<Arc<T>>;

    /// Sets the current task with given `next`.
    ///
    /// The runqueue should keep the previous current in.
    fn set_current(&mut self, next: Option<Arc<T>>);

    /// Gets the current task.
    ///
    /// If there is no current task, the method returns `None`.
    fn current(&self) -> Option<&Arc<T>>;

    /// Sets the `should_preempt` flag.
    fn set_should_preempt(&mut self, should_preempt: bool);

    /// Gets the `should_preempt` flag.
    ///
    /// The scheduler will try to preempt current task soon if this flag is set.
    fn should_preempt(&self) -> bool;
}

/// Possible triggers of an `enqueue`` action.
#[derive(PartialEq, Copy, Clone)]
pub enum EnqueueFlags {
    /// Spawn a new task.
    Spawn,
    /// Wake a sleeping task.
    Wake,
}

/// Possible triggers of an `update_current`` action.
#[derive(PartialEq, Copy, Clone)]
pub enum UpdateFlags {
    /// Timer interrupt.
    Tick,
    /// Task waiting.
    Wait,
    /// Task yielding.
    Yield,
}

/// Possible triggers of yielding.
#[derive(PartialEq, Copy, Clone)]
pub enum YieldFlags {
    /// Task about to wait.
    Wait,
    /// Task yielding.
    Yield,
    /// Task exited.
    Exit,
}

impl From<YieldFlags> for UpdateFlags {
    fn from(value: YieldFlags) -> Self {
        match value {
            YieldFlags::Wait => Self::Wait,
            YieldFlags::Yield => Self::Yield,
            YieldFlags::Exit => unreachable!(),
        }
    }
}

/// Possible actions of a rescheduling.
pub enum ReschedAction {
    /// Keep running current task and do nothing.
    DoNothing,
    /// Loop until finding a task to swap out the current.
    Retry,
    /// Switch to target task.
    SwitchTo(Arc<Task>),
}
