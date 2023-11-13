use crate::trap::disable_local;
use crate::{sync::Mutex, task::in_atomic};

use super::{
    preempt::preempt_stat,
    scheduler::GLOBAL_SCHEDULER,
    task::{context_switch, Task, TaskContext},
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

pub(crate) fn get_idle_task_ctx_ptr() -> *mut TaskContext {
    PROCESSOR.lock().idle_task_ctx_ptr()
}

fn panic_if_not_preemptible() {
    let cur_task = current_task();
    if !in_atomic() || cur_task.is_none() || cur_task.unwrap().status().is_exited() {
        return;
    }
    let (nr_lock, nr_soft_irq, nr_hard_irq, active) = preempt_stat();
    panic!(
        "The CPU could not be preempted: it was holding {} locks, {} hard irqs, {} soft irqs with active flag as {}.",
        nr_lock, nr_hard_irq, nr_soft_irq, active
    );
}

/// Switch to the next task selected by GLOBAL_SCHEDULER if it should.
pub fn schedule() {
    // let disable_irq = disable_local();
    let processor = PROCESSOR.lock();
    let mut scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    let mut should_switch = true;
    if let Some(cur_task) = processor.current() && cur_task.status().is_runnable() {
        should_switch = scheduler.should_preempt(&cur_task)
    };
    if !should_switch {
        return;
    }
    let _ = should_switch;

    let next_task = scheduler.fetch_next();
    drop(scheduler);
    drop(processor);

    match next_task {
        None => {
            // todo: idle_balance across cpus
        }
        Some(next_task) => {
            // todo: update the current_task.sleep_avg
            // panic_if_not_preemptible();
            switch_to(next_task);
        }
    }
}

#[warn(deprecated)]
fn cur_should_be_preempted() -> bool {
    if let Some(cur_task) = current_task() && cur_task.status().is_runnable() {
        return GLOBAL_SCHEDULER
            .lock_irq_disabled()
            .should_preempt(&cur_task);
    }
    false
}

#[warn(deprecated)]
pub fn preempt() {
    // disable interrupts to avoid nested preemption.
    let disable_irq = disable_local();
    if cur_should_be_preempted() {
        schedule();
    }
}

/// Switch to the given next task.
/// - If current task is none, then it will use the default task context
/// and it will not return to this function again.
/// - If current task status is exit, then it will not add to the scheduler.
///
/// After context switch, the current task of the processor
/// will be switched to the given next task.
fn switch_to(next_task: Arc<Task>) {
    let current_task_ctx = match current_task() {
        None => get_idle_task_ctx_ptr(),
        Some(cur_task) => {
            if cur_task.status().is_runnable() {
                GLOBAL_SCHEDULER
                    .lock_irq_disabled()
                    .enqueue(cur_task.clone());
            }
            &mut cur_task.inner_exclusive_access().ctx as *mut TaskContext
        }
    };

    let next_task_ctx = &next_task.inner_ctx() as *const TaskContext;
    PROCESSOR.lock().current = Some(next_task);
    unsafe {
        context_switch(current_task_ctx, next_task_ctx);
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
