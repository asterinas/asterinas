// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::{
    preempt::{
        activate_preemption, deactivate_preemption, in_atomic, is_preemptible, panic_if_in_atomic,
    },
    scheduler::{add_task, locked_global_scheduler, pick_next_task},
    task::{context_switch, NeedResched, Task, TaskContext},
};
use crate::{arch::timer::register_scheduler_tick, cpu_local, sync::SpinLock, trap::disable_local};

#[derive(Default)]
pub struct Processor {
    current: Option<Arc<Task>>,
    idle_task_ctx: TaskContext,
}

impl Processor {
    pub const fn new() -> Self {
        Self {
            current: None,
            idle_task_ctx: TaskContext::empty(),
        }
    }
    fn idle_task_ctx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_ctx as *mut _
    }
    pub fn current(&self) -> Option<Arc<Task>> {
        self.current.as_ref().map(Arc::clone)
    }
    pub fn set_current_task(&mut self, task: Arc<Task>) {
        self.current = Some(task);
    }
}

// FIXME: remove lock on the processor
cpu_local! {
    static PROCESSOR: SpinLock<Processor> = SpinLock::new(Processor::new());
}

pub fn init() {
    register_scheduler_tick(scheduler_tick);
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.lock_irq_disabled().current()
}

/// Yields execution so that another task may be scheduled.
/// Unlike in Linux, this will not change the task's status into runnable.
///
/// Note that this method cannot be simply named "yield" as the name is
/// a Rust keyword.
pub fn yield_now() {
    if current_task().is_some() {
        locked_global_scheduler().prepare_to_yield_cur_task();
    }
    schedule();
}

// FIXME: remove this func after merging #632.
pub fn yield_to(task: Arc<Task>) {
    if current_task().is_some() {
        locked_global_scheduler().prepare_to_yield_to(task);
    } else {
        add_task(task);
    }
    schedule();
}

/// Switch to the next task selected by the global scheduler if it should.
pub fn schedule() {
    if !is_preemptible() {
        panic!("schedule() is called under a non-preemptible context.");
    }
    deactivate_preemption();

    if should_preempt_cur_task() {
        switch_to_next();
    }
    activate_preemption();
}

fn switch_to_next() {
    match pick_next_task() {
        None => {
            // TODO: idle_balance across cpus
        }
        Some(next_task) => {
            switch_to(next_task);
        }
    }
}

fn should_preempt_cur_task() -> bool {
    if in_atomic() {
        return false;
    }

    current_task().map_or(true, |ref cur_task| {
        !cur_task.status().is_runnable() || cur_task.need_resched()
    }) || locked_global_scheduler().should_preempt_cur_task()
}

/// Switch to the given next task.
/// - If current task is none, then it will use the default task context
/// and it will not return to this function again.
/// - If current task status is exit, then it will not add to the scheduler.
///
/// After context switch, the current task of the processor
/// will be switched to the given next task.
///
/// This method should be called with preemption guard.
fn switch_to(next_task: Arc<Task>) {
    panic_if_in_atomic();
    let mut processor = PROCESSOR.lock();
    let current_task_ctx = match processor.current() {
        None => processor.idle_task_ctx_ptr(),
        Some(ref cur_task) => {
            if cur_task.status().is_runnable() {
                add_task(cur_task.clone());
            }
            &mut cur_task.inner_exclusive_access().ctx as *mut TaskContext
        }
    };

    let next_task_ctx = &next_task.context() as *const TaskContext;
    processor.set_current_task(next_task);
    drop(processor);
    unsafe {
        context_switch(current_task_ctx, next_task_ctx);
    }
}

/// Called by the timer handler at every TICK update.
fn scheduler_tick() {
    let disable_irq = disable_local();
    if current_task().is_some() {
        locked_global_scheduler().tick_cur_task();
    }
}
