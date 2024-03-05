// SPDX-License-Identifier: MPL-2.0

use intrusive_collections::{intrusive_adapter, LinkedListAtomicLink};

use super::{
    add_task, clear_task,
    preempt::{activate_preemption, panic_if_in_atomic},
    priority::Priority,
    processor::{current_task, schedule},
    yield_to,
};
use crate::{
    arch::mm::PageTableFlags,
    config::{KERNEL_STACK_SIZE, PAGE_SIZE},
    cpu::CpuSet,
    prelude::*,
    sync::{Mutex, MutexGuard},
    timer::current_tick,
    user::UserSpace,
    vm::{page_table::KERNEL_PAGE_TABLE, VmAllocOptions, VmSegment},
};

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

impl CalleeRegs {
    pub const fn zeros() -> Self {
        Self {
            rsp: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub rip: usize,
}

impl TaskContext {
    pub const fn empty() -> Self {
        Self {
            regs: CalleeRegs::zeros(),
            rip: 0,
        }
    }
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}

pub struct KernelStack {
    segment: VmSegment,
    old_guard_page_flag: Option<PageTableFlags>,
}

impl KernelStack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            segment: VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE)
                .is_contiguous(true)
                .alloc_contiguous()?,
            old_guard_page_flag: None,
        })
    }

    /// Generate a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.  
    pub fn new_with_guard_page() -> Result<Self> {
        let stack_segment = VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE + 1)
            .is_contiguous(true)
            .alloc_contiguous()?;
        let unpresent_flag = PageTableFlags::empty();
        let old_guard_page_flag = Self::protect_guard_page(&stack_segment, unpresent_flag);
        Ok(Self {
            segment: stack_segment,
            old_guard_page_flag: Some(old_guard_page_flag),
        })
    }

    pub fn end_paddr(&self) -> Paddr {
        self.segment.end_paddr()
    }

    pub fn has_guard_page(&self) -> bool {
        self.old_guard_page_flag.is_some()
    }

    fn protect_guard_page(stack_segment: &VmSegment, flags: PageTableFlags) -> PageTableFlags {
        let mut kernel_pt = KERNEL_PAGE_TABLE.get().unwrap().lock();
        let guard_page_vaddr = {
            let guard_page_paddr = stack_segment.start_paddr();
            crate::vm::paddr_to_vaddr(guard_page_paddr)
        };
        // Safety: The protected address must be the address of guard page hence it should be safe and valid.
        unsafe { kernel_pt.protect(guard_page_vaddr, flags).unwrap() }
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        if self.has_guard_page() {
            Self::protect_guard_page(&self.segment, self.old_guard_page_flag.unwrap());
        }
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
        core::ptr::eq(self, other)
    }
}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

pub(crate) struct TaskInner {
    pub task_status: TaskStatus,
    pub ctx: TaskContext,
    pub need_resched: bool,
    pub woken_up_timestamp: Option<u64>, // in Tick
}

impl Task {
    /// Guarded access to the task inner.
    pub(crate) fn inner_exclusive_access(&self) -> MutexGuard<'_, TaskInner> {
        self.task_inner.lock()
    }

    pub(crate) fn context(&self) -> TaskContext {
        self.task_inner.lock().ctx
    }

    pub fn run(self: &Arc<Self>) {
        debug_assert!(self.status().is_runnable());
        // FIXME: remove CHILD_RUN_FIRST after merging
        const CHILD_RUN_FIRST: bool = true;
        if CHILD_RUN_FIRST {
            yield_to(self.clone());
        } else {
            add_task(self.clone());
            schedule();
        }
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

    pub fn exit(self: &Arc<Self>) -> ! {
        self.inner_exclusive_access().task_status = TaskStatus::Exited;
        clear_task(self);
        schedule();
        panic_if_in_atomic();
        unreachable!()
    }
}

pub trait ReadPriority {
    fn priority(&self) -> Priority;

    fn is_real_time(&self) -> bool;
}
impl ReadPriority for Task {
    fn priority(&self) -> Priority {
        self.priority
    }

    fn is_real_time(&self) -> bool {
        self.priority.is_real_time()
    }
}

pub trait WakeUp {
    fn wakeup(&self);

    fn woken_up_timestamp(&self) -> Option<u64>;

    fn clear_woken_up_timestamp(&self);
}
impl WakeUp for Task {
    fn wakeup(&self) {
        let inner = &mut self.task_inner.lock();
        if inner.task_status.is_sleeping() {
            inner.task_status = TaskStatus::Runnable;
            inner.woken_up_timestamp = Some(current_tick());
        }
    }

    fn woken_up_timestamp(&self) -> Option<u64> {
        self.task_inner.lock().woken_up_timestamp
    }

    fn clear_woken_up_timestamp(&self) {
        self.task_inner.lock().woken_up_timestamp = None;
    }
}

/// To get or set the `need_resched` flag of a task.
pub trait NeedResched {
    fn need_resched(&self) -> bool;
    fn set_need_resched(&self, need_resched: bool);
}
impl NeedResched for Task {
    fn set_need_resched(&self, need_resched: bool) {
        self.inner_exclusive_access().need_resched = need_resched;
    }

    fn need_resched(&self) -> bool {
        self.inner_exclusive_access().need_resched
    }
}

pub trait Current {
    fn current() -> Arc<Self>;
}
impl Current for Task {
    fn current() -> Arc<Self> {
        current_task().unwrap()
    }
}

pub trait SchedTaskBase: ReadPriority + NeedResched + Current {}
impl SchedTaskBase for Task {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
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
        let result = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
                need_resched: false,
                woken_up_timestamp: None,
            }),
            exit_code: 0,
            kstack: KernelStack::new_with_guard_page()?,
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

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
        let result = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            task_inner: Mutex::new(TaskInner {
                task_status: TaskStatus::Runnable,
                ctx: TaskContext::default(),
                need_resched: false,
                woken_up_timestamp: None,
            }),
            exit_code: 0,
            kstack: KernelStack::new_with_guard_page()?,
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

        result.task_inner.lock().ctx.rip = kernel_task_entry as usize;
        result.task_inner.lock().ctx.regs.rsp =
            (crate::vm::paddr_to_vaddr(result.kstack.end_paddr())) as u64;

        let arc_self = Arc::new(result);
        arc_self.run();
        Ok(arc_self)
    }
}

/// All tasks will enter this function.
/// This function is meant to execute the `func` in [`Task`].
fn kernel_task_entry() {
    activate_preemption();
    let current_task =
        current_task().expect("no current task, it should have current task in kernel task entry");
    current_task.func.call(());
    current_task.exit();
}
