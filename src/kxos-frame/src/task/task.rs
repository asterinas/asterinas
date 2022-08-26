use core::cell::RefMut;
use core::mem::size_of;

use crate::mm::PhysFrame;
use crate::trap::{CalleeRegs, SyscallFrame};
use crate::user::UserSpace;
use crate::{prelude::*, UPSafeCell};

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

pub struct KernelStack {
    frame: PhysFrame,
}

impl KernelStack {
    pub fn new() -> Self {
        Self {
            frame: PhysFrame::alloc().expect("out of memory"),
        }
    }
}

/// A task that executes a function to the end.
pub struct Task {
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    task_inner: UPSafeCell<TaskInner>,
    exit_code: usize,
    /// kernel stack, note that the top is SyscallFrame
    kstack: KernelStack,
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
        fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
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
            exit_code: 0,
            kstack: KernelStack::new(),
        };

        result.task_inner.exclusive_access().task_status = TaskStatus::Runnable;
        result.task_inner.exclusive_access().ctx.rip = kernel_task_entry as usize;
        result.task_inner.exclusive_access().ctx.regs.rsp = result.kstack.frame.end_pa().kvaddr().0
            as usize
            - size_of::<usize>()
            - size_of::<SyscallFrame>();

        let arc_self = Arc::new(result);
        add_task(arc_self.clone());

        schedule();
        Ok(arc_self)
    }

    pub fn syscall_frame(&self) -> &mut SyscallFrame {
        unsafe {
            &mut *(self
                .kstack
                .frame
                .end_pa()
                .kvaddr()
                .get_mut::<SyscallFrame>() as *mut SyscallFrame)
                .sub(1)
        }
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

    pub fn exit(&self) -> ! {
        self.inner_exclusive_access().task_status = TaskStatus::Exited;
        schedule();
        unreachable!()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
