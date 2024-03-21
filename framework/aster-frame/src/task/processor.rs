// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use lazy_static::lazy_static;

use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use crate::cpu_local;

pub struct Processor {
    current: Option<Arc<Task>>,
    idle_task_cx: TaskContext,
}

impl Processor {
    pub const fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::default(),
        }
    }
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
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

cpu_local! {
    static PROCESSOR: Processor = Processor::new();
}

pub fn take_current_task() -> Option<Arc<Task>> {
    PROCESSOR.borrow().take_current()
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.borrow().current()
}

pub(crate) fn get_idle_task_cx_ptr() -> *mut TaskContext {
    PROCESSOR.borrow().get_idle_task_cx_ptr()
}

/// call this function to switch to other task by using GLOBAL_SCHEDULER
pub fn schedule() {
    if let Some(task) = fetch_task() {
        switch_to_task(task);
    }
}

pub fn preempt() {
    // TODO: Refactor `preempt` and `schedule`
    // after the Atomic mode and `might_break` is enabled.
    let Some(curr_task) = current_task() else {
        return;
    };
    let mut scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    if !scheduler.should_preempt(&curr_task) {
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
    // Safety: the `PREEMPT_COUNT` utilizes an `AtomicUsize` for its internal state, ensuring that
    // modifications are atomic and therefore free from data races.
    unsafe {
        if !PREEMPT_COUNT.borrow_unchecked().is_preemptive() {
            panic!(
                "Calling schedule() while holding {} locks",
                PREEMPT_COUNT.borrow_unchecked().num_locks()
            );
            //GLOBAL_SCHEDULER.lock_irq_disabled().enqueue(next_task);
            //return;
        }
    }
    let current_task_option = current_task();
    let next_task_cx_ptr = &next_task.inner_ctx() as *const TaskContext;
    let current_task: Arc<Task>;
    let current_task_cx_ptr = match current_task_option {
        None => PROCESSOR.borrow().get_idle_task_cx_ptr(),
        Some(current_task) => {
            if current_task.status() == TaskStatus::Runnable {
                GLOBAL_SCHEDULER
                    .lock_irq_disabled()
                    .enqueue(current_task.clone());
            }
            &mut current_task.inner_exclusive_access().ctx as *mut TaskContext
        }
    };

    // change the current task to the next task
    PROCESSOR.borrow().current = Some(next_task.clone());
    unsafe {
        context_switch(current_task_cx_ptr, next_task_cx_ptr);
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
        // The `PREEMPT_COUNT` utilizes an `AtomicUsize` for its internal state, ensuring that
        // modifications are atomic and therefore free from data races.
        unsafe {
            PREEMPT_COUNT.borrow_unchecked().incease_num_locks();
        }
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
        // The `PREEMPT_COUNT` utilizes an `AtomicUsize` for its internal state, ensuring that
        // modifications are atomic and therefore free from data races.
        unsafe {
            PREEMPT_COUNT.borrow_unchecked().decrease_num_locks();
        }
    }
}

#[must_use]
pub fn disable_preempt() -> DisablePreemptGuard {
    DisablePreemptGuard::new()
}
