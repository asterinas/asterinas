// SPDX-License-Identifier: MPL-2.0

use crate::sync::SpinLock;
use crate::task::preempt::{activate_preempt, deactivate_preempt};
use crate::task::scheduler::fetch_next_task;

use super::TaskStatus;
use super::{
    preempt::{in_atomic, panic_if_in_atomic, preemptible},
    scheduler::{add_task, locked_global_scheduler},
    task::{context_switch, Task, TaskContext},
};
use alloc::sync::Arc;

pub struct Processor {
    current: Option<Arc<Task>>,
    idle_task_cx: TaskContext,
}

impl Processor {
    pub fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::default(),
        }
    }
    fn idle_task_ctx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }
    pub fn take_current(&mut self) -> Option<Arc<Task>> {
        self.current.take()
    }
    pub fn current(&self) -> Option<Arc<Task>> {
        self.current.as_ref().map(Arc::clone)
    }
    pub fn set_current_task(&mut self, task: Arc<Task>) {
        self.current = Some(task);
    }
}

static PROCESSOR: spin::Once<SpinLock<Processor>> = spin::Once::new();

pub fn init() {
    PROCESSOR.call_once(|| SpinLock::new(Processor::new()));
}

pub fn take_current_task() -> Option<Arc<Task>> {
    unsafe { PROCESSOR.get_unchecked().lock_irq_disabled().take_current() }
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.get().unwrap().lock_irq_disabled().current()
}

#[inline]
fn current_task_irq_disabled() -> Option<Arc<Task>> {
    PROCESSOR.get().unwrap().lock().current()
}

pub(crate) fn get_idle_task_ctx_ptr() -> *mut TaskContext {
    PROCESSOR
        .get()
        .unwrap()
        .lock_irq_disabled()
        .idle_task_ctx_ptr()
}

/// Yields execution so that another task may be scheduled.
/// Unlike in Linux, this will not change the task's status into runnable.
///
/// Note that this method cannot be simply named "yield" as the name is
/// a Rust keyword.
pub fn yield_now() {
    if let Some(ref cur_task) = current_task() {
        cur_task.set_need_resched(true);
    }
    schedule();
}

/// Switch to the next task selected by the global scheduler if it should.
pub fn schedule() {
    if !preemptible() {
        // panic!("[schedule] not preemptible");
        return;
    }
    deactivate_preempt();

    if should_preempt_cur_task() {
        switch_to_next();
    }
    activate_preempt();
}

fn switch_to_next() {
    match fetch_next_task() {
        None => {
            // todo: idle_balance across cpus
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

    current_task_irq_disabled().map_or(true, |ref cur_task| {
        cur_task.status() != TaskStatus::Runnable
            || cur_task.need_resched()
            || locked_global_scheduler().should_preempt(cur_task)
    })
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
pub fn switch_to(next_task: Arc<Task>) {
    panic_if_in_atomic();
    let current_task_ctx = match current_task_irq_disabled() {
        None => PROCESSOR.get().unwrap().lock().idle_task_ctx_ptr(),
        Some(ref cur_task) => {
            if cur_task.status() == TaskStatus::Runnable {
                add_task(cur_task.clone());
            }
            &mut cur_task.inner_exclusive_access().ctx as *mut TaskContext
        }
    };

    let next_task_ctx = &next_task.inner_ctx() as *const TaskContext;
    PROCESSOR.get().unwrap().lock().set_current_task(next_task);
    unsafe {
        context_switch(current_task_ctx, next_task_ctx);
    }
}
