// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use lazy_static::lazy_static;

use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use crate::{cpu_local, sync::SpinLock};

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
    fn get_idle_task_ctx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }
    pub fn take_current(&mut self) -> Option<Arc<Task>> {
        self.current.take()
    }
    pub fn current(&self) -> Option<Arc<Task>> {
        self.current.as_ref().map(Arc::clone)
    }
    pub fn set_current_task(&mut self, task: Arc<Task>) {
        self.current = Some(task.clone());
    }
}

lazy_static! {
    static ref PROCESSOR: SpinLock<Processor> = SpinLock::new(Processor::new());
}

pub fn take_current_task() -> Option<Arc<Task>> {
    PROCESSOR.lock().take_current()
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.lock().current()
}

pub(crate) fn get_idle_task_ctx_ptr() -> *mut TaskContext {
    PROCESSOR.lock().get_idle_task_ctx_ptr()
}

/// call this function to switch to other task by using GLOBAL_SCHEDULER
pub fn schedule() {
    if let Some(task) = fetch_task() {
        switch_to_task(task);
    }
}

// TODO: This interface of this method is error prone.
// The method takes an argument for the current task to optimize its efficiency,
// but the argument provided by the caller may not be the current task, really.
// Thus, this method should be removed or reworked in the future.
pub fn preempt(task: &Arc<Task>) {
    // TODO: Refactor `preempt` and `schedule`
    // after the Atomic mode and `might_break` is enabled.
    let mut scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    if !scheduler.should_preempt(task) {
        return;
    }
    let Some(next_task) = scheduler.dequeue() else {
        return;
    };
    drop(scheduler);
    switch_to_task(next_task);
}

/// call this function to switch to other task
///
/// if current task is none, then it will use the default task context and it will not return to this function again
///
/// if current task status is exit, then it will not add to the scheduler
///
/// before context switch, current task will switch to the next task
fn switch_to_task(next_task: Arc<Task>) {
    if !PREEMPT_COUNT.is_preemptive() {
        panic!(
            "Calling schedule() while holding {} locks",
            PREEMPT_COUNT.num_locks()
        );
    }

    let current_task_ctx_ptr = match current_task() {
        None => PROCESSOR.lock().get_idle_task_ctx_ptr(),
        Some(current_task) => {
            let ctx_ptr = current_task.ctx().get();

            let mut task_inner = current_task.inner_exclusive_access();

            debug_assert_ne!(task_inner.task_status, TaskStatus::Sleeping);
            if task_inner.task_status == TaskStatus::Runnable {
                drop(task_inner);
                GLOBAL_SCHEDULER.lock_irq_disabled().enqueue(current_task);
            } else if task_inner.task_status == TaskStatus::Sleepy {
                task_inner.task_status = TaskStatus::Sleeping;
            }

            ctx_ptr
        }
    };

    let next_task_ctx_ptr = next_task.ctx().get().cast_const();

    // change the current task to the next task
    PROCESSOR.lock().current = Some(next_task.clone());

    if let Some(next_user_space) = next_task.user_space() {
        next_user_space.vm_space().activate();
    }

    // SAFETY:
    // 1. `ctx` is only used in `schedule()`. We have exclusive access to both the current task
    //    context and the next task context.
    // 2. The next task context is a valid task context.
    unsafe {
        context_switch(current_task_ctx_ptr, next_task_ctx_ptr);
    }
}

cpu_local! {
    static PREEMPT_COUNT: PreemptInfo = PreemptInfo::new();
}

/// Currently, ``PreemptInfo`` only holds the number of spin
/// locks held by the current CPU. When it has a non-zero value,
/// the CPU cannot call ``schedule()``.
struct PreemptInfo {
    num_locks: AtomicUsize,
}

impl PreemptInfo {
    const fn new() -> Self {
        Self {
            num_locks: AtomicUsize::new(0),
        }
    }

    fn incease_num_locks(&self) {
        self.num_locks.fetch_add(1, Relaxed);
    }

    fn decrease_num_locks(&self) {
        self.num_locks.fetch_sub(1, Relaxed);
    }

    fn is_preemptive(&self) -> bool {
        self.num_locks.load(Relaxed) == 0
    }

    fn num_locks(&self) -> usize {
        self.num_locks.load(Relaxed)
    }
}

/// a guard for disable preempt.
pub struct DisablePreemptGuard {
    // This private field prevents user from constructing values of this type directly.
    private: (),
}

impl !Send for DisablePreemptGuard {}

impl DisablePreemptGuard {
    fn new() -> Self {
        PREEMPT_COUNT.incease_num_locks();
        Self { private: () }
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        disable_preempt()
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        PREEMPT_COUNT.decrease_num_locks();
    }
}

#[must_use]
pub fn disable_preempt() -> DisablePreemptGuard {
    DisablePreemptGuard::new()
}
