// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

pub(crate) mod atomic_mode;
mod kernel_stack;
mod preempt;
mod processor;
pub mod scheduler;

use core::{any::Any, cell::UnsafeCell};

use kernel_stack::KernelStack;
pub(crate) use preempt::cpu_local::reset_preempt_info;
use processor::current_task;

pub use self::{
    preempt::{disable_preempt, DisabledPreemptGuard},
    scheduler::info::{AtomicCpuId, Priority, TaskScheduleInfo},
};
pub(crate) use crate::arch::task::{context_switch, TaskContext};
use crate::{cpu::CpuSet, prelude::*, user::UserSpace};

/// A task that executes a function to the end.
///
/// Each task is associated with per-task data and an optional user space.
/// If having a user space, the task can switch to the user space to
/// execute user code. Multiple tasks can share a single user space.
pub struct Task {
    func: Box<dyn Fn() + Send + Sync>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<Arc<UserSpace>>,
    ctx: UnsafeCell<TaskContext>,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,

    schedule_info: TaskScheduleInfo,
}

// SAFETY: `UnsafeCell<TaskContext>` is not `Sync`. However, we only use it in `schedule()` where
// we have exclusive access to the field.
unsafe impl Sync for Task {}

impl Task {
    /// Gets the current task.
    ///
    /// It returns `None` if the function is called in the bootstrap context.
    pub fn current() -> Option<Arc<Task>> {
        current_task()
    }

    pub(super) fn ctx(&self) -> &UnsafeCell<TaskContext> {
        &self.ctx
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        scheduler::yield_now()
    }

    /// Runs the task.
    ///
    /// BUG: This method highly depends on the current scheduling policy.
    pub fn run(self: &Arc<Self>) {
        scheduler::run_new_task(self.clone());
    }

    /// Returns the task data.
    pub fn data(&self) -> &Box<dyn Any + Send + Sync> {
        &self.data
    }

    /// Get the attached scheduling information.
    pub fn schedule_info(&self) -> &TaskScheduleInfo {
        &self.schedule_info
    }

    /// Returns the user space of this task, if it has.
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        if self.user_space.is_some() {
            Some(self.user_space.as_ref().unwrap())
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
        // `current_task()` still holds a strong reference, so nothing is destroyed at this point,
        // neither is the kernel stack.
        drop(self);
        scheduler::exit_current();
        unreachable!()
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
        Self {
            func: Some(Box::new(func)),
            data: None,
            user_space: None,
            priority: Priority::normal(),
            cpu_affinity: CpuSet::new_full(),
        }
    }

    /// Sets the function that represents the entry point of the task.
    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.func = Some(Box::new(func));
        self
    }

    /// Sets the data associated with the task.
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

    /// Sets the CPU affinity mask for the task.
    ///
    /// The `cpu_affinity` parameter represents
    /// the desired set of CPUs to run the task on.
    pub fn cpu_affinity(mut self, cpu_affinity: CpuSet) -> Self {
        self.cpu_affinity = cpu_affinity;
        self
    }

    /// Builds a new task without running it immediately.
    pub fn build(self) -> Result<Task> {
        /// all task will entering this function
        /// this function is mean to executing the task_fn in Task
        extern "C" fn kernel_task_entry() {
            let current_task = current_task()
                .expect("no current task, it should have current task in kernel task entry");
            current_task.func.call(());
            current_task.exit();
        }

        let mut new_task = Task {
            func: self.func.unwrap(),
            data: self.data.unwrap(),
            user_space: self.user_space,
            ctx: UnsafeCell::new(TaskContext::default()),
            kstack: KernelStack::new_with_guard_page()?,
            schedule_info: TaskScheduleInfo {
                cpu: AtomicCpuId::default(),
                priority: self.priority,
                cpu_affinity: self.cpu_affinity,
            },
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

        Ok(new_task)
    }

    /// Builds a new task and run it immediately.
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = Arc::new(self.build()?);
        task.run();
        Ok(task)
    }
}

/// Trait for manipulating the task context.
pub trait TaskContextApi {
    /// Sets instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Gets instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Sets stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Gets stack pointer
    fn stack_pointer(&self) -> usize;
}

#[cfg(ktest)]
mod test {
    use crate::prelude::*;

    #[ktest]
    fn create_task() {
        #[allow(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let task = Arc::new(
            crate::task::TaskOptions::new(task)
                .data(())
                .build()
                .unwrap(),
        );
        task.run();
    }

    #[ktest]
    fn spawn_task() {
        #[allow(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let _ = crate::task::TaskOptions::new(task).data(()).spawn();
    }
}
