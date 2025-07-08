// SPDX-License-Identifier: MPL-2.0

//! User mode.

use crate::{arch::trap::TrapFrame, cpu::context::UserContext};

/// Specific architectures need to implement this trait. This should only used in [`UserMode`]
///
/// Only visible in `ostd`.
pub(crate) trait UserContextApiInternal {
    /// Starts executing in the user mode.
    fn execute<F>(&mut self, has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool;

    /// Uses the information inside CpuContext to build a trapframe
    fn as_trap_frame(&self) -> TrapFrame;
}

/// The common interface that every CPU architecture-specific [`UserContext`] implements.
pub trait UserContextApi {
    /// Gets the trap number of this interrupt.
    fn trap_number(&self) -> usize;

    /// Gets the trap error code of this interrupt.
    fn trap_error_code(&self) -> usize;

    /// Sets the instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Gets the instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Sets the stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Gets the stack pointer
    fn stack_pointer(&self) -> usize;
}

/// Code execution in the user mode.
///
/// This type enables executing the code in user space from a task in the kernel
/// space safely.
///
/// Here is a sample code on how to use `UserMode`.
///  
/// ```no_run
/// use ostd::task::Task;
///
/// let current = Task::current();
/// let user_ctx = current.user_ctx()
///     .expect("the current task is not associated with a user context");
/// let mut user_mode = UserMode::new(UserContext::clone(user_ctx));
/// loop {
///     // Execute in the user space until some interesting events occur.
///     // Note: users should activate a suitable `VmSpace` before to support
///     // user-mode execution.
///     let return_reason = user_mode.execute(|| false);
///     todo!("handle the event, e.g., syscall");
/// }
/// ```
pub struct UserMode {
    context: UserContext,
}

// An instance of `UserMode` is bound to the current task. So it must not be sent to other tasks.
impl !Send for UserMode {}
// Note that implementing `!Sync` is unnecessary
// because entering the user space via `UserMode` requires taking a mutable reference.

impl UserMode {
    /// Creates a new `UserMode`.
    pub fn new(context: UserContext) -> Self {
        Self { context }
    }

    /// Starts executing in the user mode. Make sure current task is the task in `UserMode`.
    ///
    /// The method returns for one of three possible reasons indicated by [`ReturnReason`].
    /// 1. A system call is issued by the user space;
    /// 2. A CPU exception is triggered by the user space;
    /// 3. A kernel event is pending, as indicated by the given closure.
    ///
    /// After handling whatever user or kernel events that
    /// cause the method to return
    /// and updating the user-mode CPU context,
    /// this method can be invoked again to go back to the user space.
    #[track_caller]
    pub fn execute<F>(&mut self, has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        crate::task::atomic_mode::might_sleep();
        self.context.execute(has_kernel_event)
    }

    /// Returns an immutable reference the user-mode CPU context.
    pub fn context(&self) -> &UserContext {
        &self.context
    }

    /// Returns a mutable reference the user-mode CPU context.
    pub fn context_mut(&mut self) -> &mut UserContext {
        &mut self.context
    }
}

#[derive(PartialEq, Eq, Debug)]
/// A reason as to why the control of the CPU is returned from
/// the user space to the kernel.
pub enum ReturnReason {
    /// A system call is issued by the user space.
    UserSyscall,
    /// A CPU exception is triggered by the user space.
    UserException,
    /// A kernel event is pending
    KernelEvent,
}
