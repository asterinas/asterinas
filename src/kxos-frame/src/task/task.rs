use core::cell::RefMut;
use core::intrinsics::unreachable;
use core::mem::size_of;

use crate::trap::CalleeRegs;
use crate::user::UserSpace;
use crate::{prelude::*, UPSafeCell, println};

use super::processor::{current_task, schedule};
use super::scheduler::add_task;

core::arch::global_asm!(include_str!("switch.S"));
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TaskContext {
    pub regs: CalleeRegs,
    pub rip: usize,
}

extern "C" {
    pub fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
/// 8*PAGE_SIZE
#[cfg(debug_assertions)]
pub const TASK_SIZE: usize = 32768;
/// 2*PAGE_SIZE
#[cfg(not(debug_assertions))]
pub const TASK_SIZE: usize = 8192;

#[cfg(debug_assertions)]
#[repr(align(32768))]
struct TaskAlign;

#[cfg(not(debug_assertions))]
#[repr(C, align(8192))]
struct TaskAlign;

pub const KERNEL_STACK_SIZE: usize =
    TASK_SIZE - size_of::<Box<dyn Fn()>>() - size_of::<Box<dyn Any + Send + Sync>>() - size_of::<Option<Arc<UserSpace>>>()
    - size_of::<UPSafeCell<TaskInner>>() - size_of::<usize>();

/// A task that executes a function to the end.
pub struct Task {
    _align: TaskAlign,
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    task_inner: UPSafeCell<TaskInner>,
    exit_code: usize,
    kstack: [u8; KERNEL_STACK_SIZE],
}

pub struct TaskInner {
    pub task_status: TaskStatus,
    pub ctx: TaskContext,
}

impl Task {
    /// Gets the current task.
    pub fn current() -> Arc<Task> {
        current_task().unwrap()
    }

    /// get inner
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskInner> {
        self.task_inner.exclusive_access()
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        todo!()
    }

    /// Spawns a task that executes a function.
    ///
    /// Each task is associated with a per-task data and an optional user space.
    /// If having a user space, then the task can switch to the user space to
    /// execute user code. Multiple tasks can share a single user space.
    pub fn spawn<F, T>(
        task_fn: F,
        task_data: T,
        user_space: Option<Arc<UserSpace>>,
    ) -> Result<Arc<Self>>
    where
        F: Fn() + Send + Sync + 'static,
        T: Any + Send + Sync,
    {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        fn kernel_task_entry(){
            let current_task = current_task().expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(())
        }
        let result = Self {
            func: Box::new(task_fn),
            data: Box::new(task_data),
            user_space,
            task_inner: unsafe {
                UPSafeCell::new(TaskInner {
                    task_status: TaskStatus::Runnable,
                    ctx: TaskContext::default(),
                })
            },
            _align: TaskAlign,
            exit_code:0,
            kstack: [0; KERNEL_STACK_SIZE],
        };

        result.task_inner.exclusive_access().ctx.rip = kernel_task_entry as usize;
        let arc_self = Arc::new(result);

        add_task(arc_self.clone());
        schedule();
        Ok(arc_self)
    }

    /// Returns the task status.
    pub fn status(&self) -> TaskStatus {
        self.task_inner.exclusive_access().task_status
    }

    /// Returns the task data.
    pub fn data(&self) -> &dyn Any {
        &self.data
    }

    /// Returns the user space of this task, if it has.
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        if self.user_space.is_some() {
            Some(self.user_space.as_ref().unwrap())
        } else {
            None
        }
    }

    pub fn exit(&self)->!{
        self.inner_exclusive_access().task_status = TaskStatus::Exited;
        schedule();
        unreachable!()
    }
}

#[derive(Clone, Copy,PartialEq, Eq, PartialOrd, Ord)]
/// The status of a task.
pub enum TaskStatus {
    /// The task is runnable.
    Runnable,
    /// The task is running.
    Running,
    /// The task is sleeping.
    Sleeping,
    /// The task has exited.
    Exited,
}
