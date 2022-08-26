use super::{
    scheduler::{fetch_task, GLOBAL_SCHEDULER},
    task::{context_switch, TaskContext},
    Task, TaskStatus,
};
use crate::UPSafeCell;
use alloc::sync::Arc;
use lazy_static::*;

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
    pub static ref PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };
}

pub fn take_current_task() -> Option<Arc<Task>> {
    PROCESSOR.exclusive_access().take_current()
}

pub fn current_task() -> Option<Arc<Task>> {
    PROCESSOR.exclusive_access().current()
}

/// call this function to switch to other task by using GLOBAL_SCHEDULER
///
/// if current task is none, then it will use the default task context and it will not return to this function again
///
/// if current task status is exit, then it will not add to the scheduler
///
/// before context switch, current task will switch to the next task
pub fn schedule() {
    let next_task = fetch_task().expect("no more task found");
    let current_task_option = current_task();
    let next_task_cx_ptr = &next_task.inner_exclusive_access().ctx as *const TaskContext;
    let current_task: Arc<Task>;
    let current_task_cx_ptr = if current_task_option.is_none() {
        PROCESSOR.exclusive_access().get_idle_task_cx_ptr()
    } else {
        current_task = current_task_option.unwrap();
        if current_task.status() != TaskStatus::Exited {
            GLOBAL_SCHEDULER
                .exclusive_access()
                .enqueue(current_task.clone());
        }
        &mut current_task.inner_exclusive_access().ctx as *mut TaskContext
    };
    // change the current task to the next task

    PROCESSOR.exclusive_access().current = Some(next_task.clone());
    unsafe {
        context_switch(current_task_cx_ptr, next_task_cx_ptr);
    }
}
