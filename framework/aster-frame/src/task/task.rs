// SPDX-License-Identifier: MPL-2.0

use core::cell::UnsafeCell;

use intrusive_collections::{intrusive_adapter, LinkedListAtomicLink};

use super::{
    add_task,
    priority::Priority,
    processor::{current_task, schedule},
};
use crate::{
    cpu::CpuSet,
    prelude::*,
    sync::{SpinLock, SpinLockGuard},
    user::UserSpace,
    vm::{kspace::KERNEL_PAGE_TABLE, PageFlags, VmAllocOptions, VmSegment, PAGE_SIZE},
};

pub const KERNEL_STACK_SIZE: usize = PAGE_SIZE * 64;

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
    segment: VmSegment,
    has_guard_page: bool,
}

impl KernelStack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            segment: VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE).alloc_contiguous()?,
            has_guard_page: false,
        })
    }

    /// Generate a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.  
    pub fn new_with_guard_page() -> Result<Self> {
        let stack_segment =
            VmAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE + 1).alloc_contiguous()?;
        // FIXME: modifying the the linear mapping is bad.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let guard_page_vaddr = {
            let guard_page_paddr = stack_segment.start_paddr();
            crate::vm::paddr_to_vaddr(guard_page_paddr)
        };
        // SAFETY: the segment allocated is not used by others so we can protect it.
        unsafe {
            page_table
                .protect(&(guard_page_vaddr..guard_page_vaddr + PAGE_SIZE), |p| {
                    p.flags -= PageFlags::RW
                })
                .unwrap();
        }
        Ok(Self {
            segment: stack_segment,
            has_guard_page: true,
        })
    }

    pub fn end_paddr(&self) -> Paddr {
        self.segment.end_paddr()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        if self.has_guard_page {
            // FIXME: modifying the the linear mapping is bad.
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let guard_page_vaddr = {
                let guard_page_paddr = self.segment.start_paddr();
                crate::vm::paddr_to_vaddr(guard_page_paddr)
            };
            // SAFETY: the segment allocated is not used by others so we can protect it.
            unsafe {
                page_table
                    .protect(&(guard_page_vaddr..guard_page_vaddr + PAGE_SIZE), |p| {
                        p.flags |= PageFlags::RW
                    })
                    .unwrap();
            }
        }
    }
}

/// A task that executes a function to the end.
///
/// Each task is associated with per-task data and an optional user space.
/// If having a user space, the task can switch to the user space to
/// execute user code. Multiple tasks can share a single user space.
pub struct Task {
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    task_inner: SpinLock<TaskInner>,
    ctx: UnsafeCell<TaskContext>,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,
    link: LinkedListAtomicLink,
    priority: Priority,
    // TODO: add multiprocessor support
    cpu_affinity: CpuSet,
}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

// SAFETY: `UnsafeCell<TaskContext>` is not `Sync`. However, we only use it in `schedule()` where
// we have exclusive access to the field.
unsafe impl Sync for Task {}

pub(crate) struct TaskInner {
    pub task_status: TaskStatus,
}

impl Task {
    /// Gets the current task.
    pub fn current() -> Arc<Task> {
        current_task().unwrap()
    }

    /// get inner
    pub(crate) fn inner_exclusive_access(&self) -> SpinLockGuard<TaskInner> {
        self.task_inner.lock_irq_disabled()
    }

    pub(super) fn ctx(&self) -> &UnsafeCell<TaskContext> {
        &self.ctx
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        schedule();
    }

    /// Runs the task.
    pub fn run(self: &Arc<Self>) {
        add_task(self.clone());
        schedule();
    }

    /// Returns the task status.
    pub fn status(&self) -> TaskStatus {
        self.task_inner.lock_irq_disabled().task_status
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
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
/// The status of a task.
pub enum TaskStatus {
    /// The task is runnable.
    Runnable,
    /// The task is running in the foreground but will sleep when it goes to the background.
    Sleepy,
    /// The task is sleeping in the background.
    Sleeping,
    /// The task has exited.
    Exited,
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

    /// Build a new task without running it immediately.
    pub fn build(self) -> Result<Arc<Task>> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        extern "sysv64" fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(());
            current_task.exit();
        }

        let mut new_task = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            task_inner: SpinLock::new(TaskInner {
                task_status: TaskStatus::Runnable,
            }),
            ctx: UnsafeCell::new(TaskContext::default()),
            kstack: KernelStack::new_with_guard_page()?,
            link: LinkedListAtomicLink::new(),
            priority: self.priority,
            cpu_affinity: self.cpu_affinity,
        };

        let ctx = new_task.ctx.get_mut();
        ctx.rip = kernel_task_entry as usize;
        // We should reserve space for the return address in the stack, otherwise
        // we will write across the page boundary due to the implementation of
        // the context switch.
        //
        // According to the System V AMD64 ABI, the stack pointer should be aligned
        // to at least 16 bytes. And a larger alignment is needed if larger arguments
        // are passed to the function. The `kernel_task_entry` function does not
        // have any arguments, so we only need to align the stack pointer to 16 bytes.
        ctx.regs.rsp = (crate::vm::paddr_to_vaddr(new_task.kstack.end_paddr() - 16)) as u64;

        Ok(Arc::new(new_task))
    }

    /// Build a new task and run it immediately.
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = self.build()?;
        task.run();
        Ok(task)
    }
}

#[cfg(ktest)]
mod test {
    #[ktest]
    fn create_task() {
        let task = || {
            assert_eq!(1, 1);
        };
        let task_option = crate::task::TaskOptions::new(task)
            .data(())
            .build()
            .unwrap();
        task_option.run();
    }

    #[ktest]
    fn spawn_task() {
        let task = || {
            assert_eq!(1, 1);
        };
        let _ = crate::task::TaskOptions::new(task).data(()).spawn();
    }
}
