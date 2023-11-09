use super::preempt_stat;
use crate::trap::disable_local;
use crate::{sync::Mutex, task::in_atomic};

use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use alloc::sync::Arc;
use lazy_static::lazy_static;

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

lazy_static! {
    static ref PROCESSOR: Mutex<Processor> = Mutex::new(Processor::new());
}

pub fn take_current_task() -> Option<Arc<Task>> {
    PROCESSOR.lock().take_current()
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.lock().current()
}

pub(crate) fn get_idle_task_cx_ptr() -> *mut TaskContext {
    PROCESSOR.lock().get_idle_task_cx_ptr()
}

fn panic_if_not_preemptible() {
    if !in_atomic() {
        return;
    }
    let (nr_lock, nr_soft_irq, nr_hard_irq, active) = preempt_stat();
    panic!(
        "The CPU could not be preempted: it was holding {} locks, {} hard irqs, {} soft irqs with active as {}.",
        nr_lock, nr_hard_irq, nr_soft_irq, active
    );
}

/// call this function to switch to other task by using GLOBAL_SCHEDULER
pub fn schedule() {
    // todo: preempt_disable
    let task = fetch_task();
    if task.is_none() {
        return;
    };
    let task = task.unwrap();
    // panic_if_not_preemptible();
    switch_to_task(task);
}

fn cur_should_be_preempted() -> bool {
    if let Some(cur_task) = current_task() && cur_task.status().is_runnable() {
        return GLOBAL_SCHEDULER
            .lock_irq_disabled()
            .should_preempt(&cur_task);
    }
    false
}

pub fn preempt() {
    // disable interrupts to avoid nested preemption.
    let disable_irq = disable_local();
    if cur_should_be_preempted() {
        schedule();
    }
}

/// call this function to switch to other task
///
/// if current task is none, then it will use the default task context and it will not return to this function again
///
/// if current task status is exit, then it will not add to the scheduler
///
/// before context switch, current task will switch to the next task
fn switch_to_task(next_task: Arc<Task>) {
    // here was a preemptible check => may cause bug?(panic)

    let current_task_option = current_task();
    let next_task_cx_ptr = &next_task.inner_ctx() as *const TaskContext;
    let current_task: Arc<Task>;
    let current_task_cx_ptr = match current_task_option {
        None => PROCESSOR.lock().get_idle_task_cx_ptr(),
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

    PROCESSOR.lock().current = Some(next_task.clone());
    unsafe {
        context_switch(current_task_cx_ptr, next_task_cx_ptr);
    }
}

/// Called by the timer handler every TICK.
///
/// # Arguments
///
/// * `cur_tick` - The current tick count.
pub(crate) fn scheduler_tick(cur_tick: u64) {
    let processor = PROCESSOR.lock();
    let Some(cur_task) = processor.current() else {
        return;
    };
    // update_cpu_clock(p, rq, now);
    GLOBAL_SCHEDULER
        .lock_irq_disabled()
        .tick(cur_task.clone(), cur_tick);
}
