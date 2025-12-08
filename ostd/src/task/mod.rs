// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

pub mod atomic_mode;
mod kernel_stack;
mod preempt;
mod processor;
pub mod scheduler;
mod utils;

use core::{
    any::Any,
    borrow::Borrow,
    cell::{Cell, SyncUnsafeCell},
    ops::Deref,
    ptr::NonNull,
    sync::atomic::AtomicBool,
};

use kernel_stack::KernelStack;
use processor::current_task;
use spin::Once;
use utils::ForceSync;

pub use self::{
    preempt::{DisabledPreemptGuard, disable_preempt, halt_cpu},
    scheduler::info::{AtomicCpuId, TaskScheduleInfo},
};
use crate::{arch::task::TaskContext, irq::InterruptLevel, prelude::*};

static PRE_SCHEDULE_HANDLER: Once<fn()> = Once::new();

static POST_SCHEDULE_HANDLER: Once<fn()> = Once::new();

/// Injects a handler to be executed before scheduling.
pub fn inject_pre_schedule_handler(handler: fn()) {
    PRE_SCHEDULE_HANDLER.call_once(|| handler);
}

/// Injects a handler to be executed after scheduling.
pub fn inject_post_schedule_handler(handler: fn()) {
    POST_SCHEDULE_HANDLER.call_once(|| handler);
}

/// A task that executes a function to the end.
///
/// Each task is associated with per-task data and an optional user space.
/// If having a user space, the task can switch to the user space to
/// execute user code. Multiple tasks can share a single user space.
#[derive(Debug)]
pub struct Task {
    #[expect(clippy::type_complexity)]
    func: ForceSync<Cell<Option<Box<dyn FnOnce() + Send>>>>,

    data: Box<dyn Any + Send + Sync>,
    local_data: ForceSync<Box<dyn Any + Send>>,

    ctx: SyncUnsafeCell<TaskContext>,
    /// kernel stack, note that the top is SyscallFrame/TrapFrame
    kstack: KernelStack,

    /// If we have switched this task to a CPU.
    ///
    /// This is to enforce not context switching to an already running task.
    /// See [`processor::switch_to_task`] for more details.
    switched_to_cpu: AtomicBool,

    schedule_info: TaskScheduleInfo,
}

impl Task {
    /// Gets the current task.
    ///
    /// It returns `None` if the function is called in the bootstrap context.
    pub fn current() -> Option<CurrentTask> {
        let current_task = current_task()?;

        // SAFETY: `current_task` is the current task.
        Some(unsafe { CurrentTask::new(current_task) })
    }

    pub(super) fn ctx(&self) -> &SyncUnsafeCell<TaskContext> {
        &self.ctx
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    #[track_caller]
    pub fn yield_now() {
        scheduler::yield_now()
    }

    /// Kicks the task scheduler to run the task.
    ///
    /// BUG: This method highly depends on the current scheduling policy.
    #[track_caller]
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
}

/// Options to create or spawn a new task.
pub struct TaskOptions {
    func: Option<Box<dyn FnOnce() + Send>>,
    data: Option<Box<dyn Any + Send + Sync>>,
    local_data: Option<Box<dyn Any + Send>>,
}

impl TaskOptions {
    /// Creates a set of options for a task.
    pub fn new<F>(func: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            func: Some(Box::new(func)),
            data: None,
            local_data: None,
        }
    }

    /// Sets the function that represents the entry point of the task.
    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + 'static,
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

    /// Sets the local data associated with the task.
    pub fn local_data<T>(mut self, data: T) -> Self
    where
        T: Any + Send,
    {
        self.local_data = Some(Box::new(data));
        self
    }

    /// Builds a new task without running it immediately.
    pub fn build(self) -> Result<Task> {
        // All tasks will enter this function. It is meant to execute the `task_fn` in `Task`.
        //
        // We provide an assembly wrapper for this function as the end of call stack so we
        // have to disable name mangling for it.
        //
        // # Safety
        //
        // This function must be called from `switch.S` when the context is prepared correctly.
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        unsafe extern "C" fn kernel_task_entry() -> ! {
            // SAFETY: The new task is switched on a CPU for the first time, `after_switching_to`
            // hasn't been called yet.
            unsafe { processor::after_switching_to() };

            let current_task = Task::current()
                .expect("no current task, it should have current task in kernel task entry");

            // SAFETY: The `func` field will only be accessed by the current task in the task
            // context, so the data won't be accessed concurrently.
            let task_func = unsafe { current_task.func.get() };
            let task_func = task_func
                .take()
                .expect("task function is `None` when trying to run");
            task_func();

            // Manually drop all the on-stack variables to prevent memory leakage!
            // This is needed because `scheduler::exit_current()` will never return.
            //
            // However, `current_task` _borrows_ the current task without holding
            // an extra reference count. So we do nothing here.

            scheduler::exit_current();
        }

        let kstack = KernelStack::new_with_guard_page()?;

        let mut ctx = TaskContext::new();
        ctx.set_instruction_pointer(crate::arch::task::kernel_task_entry_wrapper as usize);
        // We should reserve space for the return address in the stack, otherwise
        // we will write across the page boundary due to the implementation of
        // the context switch.
        //
        // According to the System V AMD64 ABI, the stack pointer should be aligned
        // to at least 16 bytes. And a larger alignment is needed if larger arguments
        // are passed to the function. The `kernel_task_entry` function does not
        // have any arguments, so we only need to align the stack pointer to 16 bytes.
        ctx.set_stack_pointer(kstack.end_vaddr() - 16);

        let new_task = Task {
            func: ForceSync::new(Cell::new(self.func)),
            data: self.data.unwrap_or_else(|| Box::new(())),
            local_data: ForceSync::new(self.local_data.unwrap_or_else(|| Box::new(()))),
            ctx: SyncUnsafeCell::new(ctx),
            kstack,
            schedule_info: TaskScheduleInfo {
                cpu: AtomicCpuId::default(),
            },
            switched_to_cpu: AtomicBool::new(false),
        };

        Ok(new_task)
    }

    /// Builds a new task and runs it immediately.
    #[track_caller]
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = Arc::new(self.build()?);
        task.run();
        Ok(task)
    }
}

/// The current task.
///
/// This type is not `Send`, so it cannot outlive the current task.
///
/// This type is also not `Sync`, so it can provide access to the local data of the current task.
#[derive(Debug)]
pub struct CurrentTask(NonNull<Task>);

// The intern `NonNull<Task>` contained by `CurrentTask` implies that `CurrentTask` is `!Send` and
// `!Sync`. But it is still good to do this explicitly because these properties are key for
// soundness.
impl !Send for CurrentTask {}
impl !Sync for CurrentTask {}

impl CurrentTask {
    /// # Safety
    ///
    /// The caller must ensure that `task` is the current task.
    unsafe fn new(task: NonNull<Task>) -> Self {
        Self(task)
    }

    /// Returns the local data of the current task.
    ///
    /// Note that the local data is only accessible in the task context. Although there is a
    /// current task in the non-task context (e.g. IRQ handlers), access to the local data is
    /// forbidden as it may cause soundness problems.
    ///
    /// # Panics
    ///
    /// This method will panic if called in a non-task context.
    pub fn local_data(&self) -> &(dyn Any + Send) {
        assert!(InterruptLevel::current().is_task_context());

        let local_data = &self.local_data;

        // SAFETY: The `local_data` field will only be accessed by the current task in the task
        // context, so the data won't be accessed concurrently.
        &**unsafe { local_data.get() }
    }

    /// Returns a cloned `Arc<Task>`.
    pub fn cloned(&self) -> Arc<Task> {
        let ptr = self.0.as_ptr();

        // SAFETY: The current task is always a valid task and it is always contained in an `Arc`.
        unsafe { Arc::increment_strong_count(ptr) };

        // SAFETY: We've increased the reference count in the current `Arc<Task>` above.
        unsafe { Arc::from_raw(ptr) }
    }
}

impl Deref for CurrentTask {
    type Target = Task;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The current task is always a valid task.
        unsafe { self.0.as_ref() }
    }
}

impl AsRef<Task> for CurrentTask {
    fn as_ref(&self) -> &Task {
        self
    }
}

impl Borrow<Task> for CurrentTask {
    fn borrow(&self) -> &Task {
        self
    }
}

/// A trait that provides methods to manipulate the task context.
pub(crate) trait TaskContextApi {
    /// Sets the instruction pointer.
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Sets the stack pointer.
    fn set_stack_pointer(&mut self, sp: usize);
}

#[cfg(ktest)]
mod test {
    use crate::prelude::*;

    #[ktest]
    fn create_task() {
        #[expect(clippy::eq_op)]
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
        #[expect(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let _ = crate::task::TaskOptions::new(task).data(()).spawn();
    }
}
