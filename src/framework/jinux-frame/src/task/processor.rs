use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use alloc::sync::Arc;
use lazy_static::lazy_static;
use spin::Mutex;

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

/// call this function to switch to other task by using GLOBAL_SCHEDULER
pub fn schedule() {
    if let Some(task) = fetch_task() {
        switch_to_task(task);
    }
}

/// call this function to switch to other task
///
/// if current task is none, then it will use the default task context and it will not return to this function again
///
/// if current task status is exit, then it will not add to the scheduler
///
/// before context switch, current task will switch to the next task
pub fn switch_to_task(next_task: Arc<Task>) {
    let current_task_option = current_task();
    let next_task_cx_ptr = &next_task.inner_ctx() as *const TaskContext;
    let current_task: Arc<Task>;
    let current_task_cx_ptr = if current_task_option.is_none() {
        PROCESSOR.lock().get_idle_task_cx_ptr()
    } else {
        current_task = current_task_option.unwrap();
        if current_task.status() != TaskStatus::Exited {
            GLOBAL_SCHEDULER.lock().enqueue(current_task.clone());
        }
        &mut current_task.inner_exclusive_access().ctx as *mut TaskContext
    };
    // change the current task to the next task

    PROCESSOR.lock().current = Some(next_task.clone());
    unsafe {
        context_switch(current_task_cx_ptr, next_task_cx_ptr);
    }
}
