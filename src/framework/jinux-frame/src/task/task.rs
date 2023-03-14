use spin::{Mutex, MutexGuard};

use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE};
use crate::prelude::*;
use crate::task::processor::switch_to_task;
use crate::user::UserSpace;
use crate::vm::{VmAllocOptions, VmFrameVec};

use intrusive_collections::intrusive_adapter;
use intrusive_collections::LinkedListAtomicLink;

use super::processor::{current_task, schedule};

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    pub rsp: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub rip: usize,
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}

pub struct KernelStack {
    frame: VmFrameVec,
}

impl KernelStack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            frame: VmFrameVec::allocate(
                &VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE).is_contiguous(true),
            )?,
        })
    }
}

/// A task that executes a function to the end.
pub struct Task {
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    task_inner: Mutex<TaskInner>,
    exit_code: usize,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,
    link: LinkedListAtomicLink,
}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

pub(crate) struct TaskInner {
    pub task_status: TaskStatus,
    pub ctx: TaskContext,
}

impl Task {
    /// Gets the current task.
    pub fn current() -> Arc<Task> {
        current_task().unwrap()
    }

    /// get inner
    pub(crate) fn inner_exclusive_access(&self) -> MutexGuard<'_, TaskInner> {
        self.task_inner.lock()
    }

    /// get inner
    pub(crate) fn inner_ctx(&self) -> TaskContext {
        self.task_inner.lock().ctx
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        schedule();
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
            current_task.func.call(());
            current_task.exit();
        }
        let result = Self {
            func: Box::new(task_fn),
            data: Box::new(task_data),
            user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
            }),
            exit_code: 0,
            kstack: KernelStack::new()?,
            link: LinkedListAtomicLink::new(),
        };

        result.task_inner.lock().task_status = TaskStatus::Runnable;
        result.task_inner.lock().ctx.rip = kernel_task_entry as usize;
        result.task_inner.lock().ctx.regs.rsp =
            (result.kstack.frame.end_pa().unwrap().kvaddr().0) as u64;

        let arc_self = Arc::new(result);
        switch_to_task(arc_self.clone());
        Ok(arc_self)
    }

    pub fn new<F, T>(
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
            current_task.func.call(());
            current_task.exit();
        }
        let result = Self {
            func: Box::new(task_fn),
            data: Box::new(task_data),
            user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
            }),
            exit_code: 0,
            kstack: KernelStack::new()?,
            link: LinkedListAtomicLink::new(),
        };

        result.task_inner.lock().task_status = TaskStatus::Runnable;
        result.task_inner.lock().ctx.rip = kernel_task_entry as usize;
        result.task_inner.lock().ctx.regs.rsp =
            (result.kstack.frame.end_pa().unwrap().kvaddr().0) as u64;

        Ok(Arc::new(result))
    }

    pub fn run(self: &Arc<Self>) {
        switch_to_task(self.clone());
    }

    /// Returns the task status.
    pub fn status(&self) -> TaskStatus {
        self.task_inner.lock().task_status
    }

    /// Returns the task data.
    pub fn data(&self) -> &Box<dyn Any + Send + Sync> {
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
