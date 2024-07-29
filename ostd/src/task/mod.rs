// SPDX-License-Identifier: MPL-2.0

// FIXME: the `intrusive_adapter` macro will generate methods without docs.
// So we temporary allow missing_docs for this module.
#![allow(missing_docs)]

//! Tasks are the unit of code execution.

mod kernel_stack;
mod priority;
mod processor;
mod scheduler;

use core::cell::UnsafeCell;

use intrusive_collections::{intrusive_adapter, LinkedListAtomicLink};
use kernel_stack::KernelStack;

pub use self::{
    priority::Priority,
    processor::{current_task, disable_preempt, schedule, DisablePreemptGuard},
    scheduler::{add_task, set_scheduler, FifoScheduler, Scheduler},
};
use crate::{
    arch::task::TaskContext as ArchTaskContext, cpu::CpuSet, prelude::*, sync::SpinLock,
    user::UserSpace,
};

/// Trait for manipulating the task context.
///
/// This is a low-level API adapter for the architectural-specific task context.
pub(crate) trait TaskContextApi {
    /// Sets instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Gets instruction pointer
    #[allow(dead_code)]
    fn instruction_pointer(&self) -> usize;

    /// Sets stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Gets stack pointer
    #[allow(dead_code)]
    fn stack_pointer(&self) -> usize;
}

/// The entrypoint function of a task takes three arguments:
///  1. the task context,
///  2. the reference to the mutable data,
///  3. and the reference to the shared data.
pub trait TaskFn =
    Fn(&mut MutTaskInfo, &SharedTaskInfo, &mut dyn Any, &(dyn Any + Send + Sync)) + 'static;

/// A task that executes a function to the end.
///
/// Each task is associated with per-task data and an optional user space.
/// If having a user space, the task can switch to the user space to
/// execute user code. Multiple tasks can share a single user space.
///
/// Please use a [`TaskOptions`] to create a task.
pub struct Task {
    // Private fields
    func: Box<dyn TaskFn>,
    /// The data that is exclusively mutable by the current task. Other tasks
    /// will not be able to access such data.
    mut_data: UnsafeCell<Box<dyn Any>>,
    /// The data that is shared among tasks. It is immutable. Any inner-mutablity
    /// would require locks.
    shared_data: Box<dyn Any + Send + Sync>,
    ctx: UnsafeCell<ArchTaskContext>,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,
    link: LinkedListAtomicLink,

    /// Other shared fields that are not related to the task's execution.
    shared_inner: SharedTaskInfo,
    /// Other fields that are exclusively mutable by the current task.
    mut_inner: UnsafeCell<MutTaskInfo>,
}

/// The extra public information of a task.
///
/// These information can be safely accessed by other tasks.
pub struct SharedTaskInfo {
    pub user_space: Option<Arc<UserSpace>>,
    pub priority: Priority,
    pub(crate) status: SpinLock<TaskStatus>,
    // TODO: add multiprocessor support
    #[allow(dead_code)]
    pub cpu_affinity: CpuSet,
}

pub struct MutTaskInfo;

impl MutTaskInfo {
    /// Preempts the current task.
    pub fn preempt(&mut self, info: &SharedTaskInfo) {
        processor::preempt(info);
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now(&mut self) {
        schedule();
    }
}

// SAFETY: It is safe to send a task between threads because the only three
// non-thread-safe part, `mut_data`, `ctx` and `MutTaskInfo` should only be
// accessed through exclusively.
unsafe impl Send for Task {}
// SAFETY: `&Task` is [`Send`] because a shared reference to a task cannot
// access it's `mut_data` and `ctx` part.
unsafe impl Sync for Task {}

// TaskAdapter struct is implemented for building relationships between doubly linked list and Task struct
intrusive_adapter!(pub TaskAdapter = Arc<Task>: Task { link: LinkedListAtomicLink });

impl Task {
    /// Gets the current task.
    pub fn current() -> Arc<Task> {
        current_task().unwrap()
    }

    pub fn priority(&self) -> Priority {
        self.shared_inner.priority
    }

    pub(crate) fn status(&self) -> &SpinLock<TaskStatus> {
        &self.shared_inner.status
    }

    pub(super) fn ctx(&self) -> &UnsafeCell<ArchTaskContext> {
        &self.ctx
    }

    /// Runs the task.
    pub fn run(self: &Arc<Self>) {
        add_task(self.clone());
        schedule();
    }

    /// Returns the task's shared data.
    pub fn shared_data(&self) -> &(dyn Any + Send + Sync) {
        self.shared_data.as_ref()
    }

    /// Returns the task's exclusive data.
    pub fn mut_data(&mut self) -> &mut dyn Any {
        self.mut_data.get_mut().as_mut()
    }

    /// Returns the user space of this task, if it has.
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        if self.shared_inner.user_space.is_some() {
            Some(self.shared_inner.user_space.as_ref().unwrap())
        } else {
            None
        }
    }

    /// Exits the current task.
    ///
    /// The task `self` must be the task that is currently running.
    ///
    /// **NOTE:** If there is anything left on the stack, it will be forgotten. This behavior may
    /// lead to resource leakage.
    fn exit(self: Arc<Self>) -> ! {
        *self.shared_inner.status.lock_irq_disabled() = TaskStatus::Exited;

        // `current_task()` still holds a strong reference, so nothing is destroyed at this point,
        // neither is the kernel stack.
        drop(self);

        schedule();
        unreachable!()
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
    func: Option<Box<dyn TaskFn>>,
    shared_data: Option<Box<dyn Any + Send + Sync>>,
    mut_data: Option<Box<dyn Any>>,
    user_space: Option<Arc<UserSpace>>,
    priority: Priority,
    cpu_affinity: CpuSet,
}

impl TaskOptions {
    /// Creates a set of options for a task.
    pub fn new(func: impl TaskFn) -> Self {
        let cpu_affinity = CpuSet::new_full();
        Self {
            func: Some(Box::new(func)),
            shared_data: None,
            mut_data: None,
            user_space: None,
            priority: Priority::normal(),
            cpu_affinity,
        }
    }

    /// Sets the function that represents the entry point of the task.
    pub fn func(mut self, func: impl TaskFn) -> Self {
        self.func = Some(Box::new(func));
        self
    }

    /// Sets the shared data associated with the task.
    pub fn shared_data<T>(mut self, data: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.shared_data = Some(Box::new(data));
        self
    }

    /// Sets the exlusive data associated with the task.
    pub fn mut_data<T>(mut self, data: T) -> Self
    where
        T: Any,
    {
        self.mut_data = Some(Box::new(data));
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

    /// Sets the CPU affinity mask for the task.
    ///
    /// The `cpu_affinity` parameter represents
    /// the desired set of CPUs to run the task on.
    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    /// Builds a new task without running it immediately.
    pub fn build(self) -> Result<Arc<Task>> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        extern "C" fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");

            // SAFETY: the task function exclusively accesses the task's
            // mutable data. No other references to the mutable data can
            // be created through a shared reference to the task.
            let mutable_info = unsafe { &mut *current_task.mut_inner.get() };
            let shared_info = &current_task.shared_inner;
            // SAFETY: the task function exclusively accesses the task's
            // mutable data. No other references to the mutable data can
            // be created through a shared reference to the task.
            let mutable_data = unsafe { &mut *current_task.mut_data.get() }.as_mut();
            let shared_data = current_task.shared_data();
            current_task
                .func
                .call((mutable_info, shared_info, mutable_data, shared_data));
            current_task.exit();
        }

        let mut new_task = Task {
            func: self.func.unwrap(),
            shared_data: self.shared_data.unwrap_or(Box::new(())),
            mut_data: UnsafeCell::new(self.mut_data.unwrap_or(Box::new(()))),
            ctx: UnsafeCell::new(ArchTaskContext::default()),
            kstack: KernelStack::new_with_guard_page()?,
            link: LinkedListAtomicLink::new(),

            shared_inner: SharedTaskInfo {
                user_space: self.user_space,
                status: SpinLock::new(TaskStatus::Runnable),
                priority: self.priority,
                cpu_affinity: self.cpu_affinity,
            },
            mut_inner: UnsafeCell::new(MutTaskInfo),
        };

        let ctx = new_task.ctx.get_mut();
        ctx.set_instruction_pointer(kernel_task_entry as usize);
        // We should reserve space for the return address in the stack, otherwise
        // we will write across the page boundary due to the implementation of
        // the context switch.
        //
        // According to the System V AMD64 ABI, the stack pointer should be aligned
        // to at least 16 bytes. And a larger alignment is needed if larger arguments
        // are passed to the function. The `kernel_task_entry` function does not
        // have any arguments, so we only need to align the stack pointer to 16 bytes.
        ctx.set_stack_pointer(crate::mm::paddr_to_vaddr(new_task.kstack.end_paddr() - 16));

        Ok(Arc::new(new_task))
    }

    /// Builds a new task and run it immediately.
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = self.build()?;
        task.run();
        Ok(task)
    }
}

#[cfg(ktest)]
mod test {
    use crate::prelude::*;

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
