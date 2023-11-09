use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE};
use crate::cpu::CpuSet;
use crate::prelude::*;
use crate::user::UserSpace;
use crate::vm::{VmAllocOptions, VmFrameVec};
use spin::{Mutex, MutexGuard};

use intrusive_collections::intrusive_adapter;
use intrusive_collections::LinkedListAtomicLink;

use super::add_task;
use super::priority::Priority;
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
                VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE).is_contiguous(true),
            )?,
        })
    }

    pub fn end_paddr(&self) -> Paddr {
        self.frame.get(self.frame.len() - 1).unwrap().end_paddr()
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
    priority: Priority,
    // TODO:: add multiprocessor support
    cpu_affinity: CpuSet,
}

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self as *const _ == other as *const _
    }
}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

pub(crate) struct TaskInner {
    pub task_status: TaskStatus,
    pub ctx: TaskContext,
    pub need_resched: bool,

    // For O(1) scheduler
    // pub avg_sleep_time: u64,
    pub dyn_prio: Priority,
    pub first_time_slice: bool,
    pub time_slice: u64,
    pub active: bool,
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

    pub fn run(self: &Arc<Self>) {
        add_task(self.clone());
        schedule();
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

    pub fn is_real_time(&self) -> bool {
        self.priority.is_real_time()
        // todo: replaced with the dynamic one?
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    pub fn dyn_prio(&self) -> Priority {
        self.inner_exclusive_access().dyn_prio
    }

    pub fn set_dyn_prio(&self, dyn_prio: Priority) {
        self.inner_exclusive_access().dyn_prio = dyn_prio;
    }

    pub fn set_need_resched(&self) {
        self.inner_exclusive_access().need_resched = true;
    }

    pub fn need_resched(&self) -> bool {
        self.inner_exclusive_access().need_resched
    }

    pub fn deny_first_time_slice(&self) {
        self.inner_exclusive_access().first_time_slice = false;
    }

    pub fn is_first_time_slice(&self) -> bool {
        self.inner_exclusive_access().first_time_slice
    }

    pub fn set_time_slice(&self, time_slice: u64) {
        self.inner_exclusive_access().time_slice = time_slice;
    }

    pub fn time_slice(&self) -> u64 {
        self.inner_exclusive_access().time_slice
    }

    pub fn set_active(&self, active_or_expired: bool) {
        self.inner_exclusive_access().active = active_or_expired;
    }

    pub fn is_active(&self) -> bool {
        self.inner_exclusive_access().active
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// The status of a task.
pub enum TaskStatus {
    /// The task is runnable.
    Runnable,
    /// The task is sleeping.
    Sleeping,
    /// The task has exited.
    Exited,
}

impl TaskStatus {
    pub fn is_runnable(&self) -> bool {
        self == &TaskStatus::Runnable
    }

    pub fn is_sleeping(&self) -> bool {
        self == &TaskStatus::Sleeping
    }

    pub fn is_exited(&self) -> bool {
        self == &TaskStatus::Exited
    }
}

/// Options to create or spawn a new task.
pub struct TaskOptions {
    func: Option<Box<dyn Fn() + Send + Sync>>,
    data: Option<Box<dyn Any + Send + Sync>>,
    user_space: Option<Arc<UserSpace>>,
    priority: Priority,
    cpu_affinity: CpuSet,
}

impl TaskOptions {
    /// Creates a set of options for a task.
    pub fn new<F>(func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let cpu_affinity = CpuSet::new_full();
        Self {
            func: Some(Box::new(func)),
            data: None,
            user_space: None,
            priority: Priority::normal(),
            cpu_affinity,
        }
    }

    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.func = Some(Box::new(func));
        self
    }

    pub fn data<T>(mut self, data: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.data = Some(Box::new(data));
        self
    }

    /// Sets the user space associated with the task.
    pub fn user_space(mut self, user_space: Option<Arc<UserSpace>>) -> Self {
        self.user_space = user_space;
        self
    }

    /// Sets the priority of the task.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    /// Builds a new task but not run it immediately.
    pub fn build(self) -> Result<Arc<Task>> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(());
            current_task.exit();
        }
        let result = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
                need_resched: false,
                dyn_prio: self.priority, // todo: calculate the dynamic priority
                // avg_sleep_time: 0,
                time_slice: 0,
                first_time_slice: true,
                active: false,
            }),
            exit_code: 0,
            kstack: KernelStack::new()?,
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

        result.task_inner.lock().task_status = TaskStatus::Runnable;
        result.task_inner.lock().ctx.rip = kernel_task_entry as usize;
        result.task_inner.lock().ctx.regs.rsp =
            (crate::vm::paddr_to_vaddr(result.kstack.end_paddr())) as u64;

        Ok(Arc::new(result))
    }

    /// Builds a new task and run it immediately.
    ///
    /// Each task is associated with a per-task data and an optional user space.
    /// If having a user space, then the task can switch to the user space to
    /// execute user code. Multiple tasks can share a single user space.
    pub fn spawn(self) -> Result<Arc<Task>> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(());
            current_task.exit();
        }
        let result = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
                need_resched: false,
                dyn_prio: self.priority, // todo: calculate the dynamic priority
                // avg_sleep_time: 0, // ?
                time_slice: 0,
                first_time_slice: true,
                active: false,
            }),
            exit_code: 0,
            kstack: KernelStack::new()?,
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

        result.task_inner.lock().task_status = TaskStatus::Runnable;
        result.task_inner.lock().ctx.rip = kernel_task_entry as usize;
        result.task_inner.lock().ctx.regs.rsp =
            (crate::vm::paddr_to_vaddr(result.kstack.end_paddr())) as u64;

        let arc_self = Arc::new(result);
        arc_self.run();
        Ok(arc_self)
    }
}
