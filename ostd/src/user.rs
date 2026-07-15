// SPDX-License-Identifier: MPL-2.0

//! User mode.

use crate::{
    arch::{cpu::context::UserContext, trap::TrapFrame},
    irq::DisabledLocalIrqGuard,
};

/// The internal interface that every CPU architecture-specific [`UserContext`] implements.
///
/// This should only used in [`UserMode`]. It is only visible in `ostd`.
pub(crate) trait UserContextApiInternal {
    /// Starts executing in the user mode.
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason;

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
/// # fn handle_return(_reason: crate::user::ReturnReason) {}
/// #
/// use crate::{
///     arch::cpu::context::UserContext,
///     user::{DummyUserHooks, UserMode},
/// };
///
/// let user_ctx = UserContext::default();
/// let mut user_mode = UserMode::new(user_ctx);
///
/// // Note: Users should activate a suitable `VmSpace` before to support
/// // user-mode execution.
///
/// loop {
///     // Execute in the user space until some interesting events occur.
///     let return_reason = user_mode.execute(&DummyUserHooks);
///     // Handle the event, e.g., a system call.
///     handle_return(return_reason);
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
    pub fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        crate::task::atomic_mode::might_sleep();
        self.context.execute(hooks)
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

/// A reason as to why the control of the CPU is returned from
/// the user space to the kernel.
#[derive(Debug, Eq, PartialEq)]
pub enum ReturnReason {
    /// A system call is issued by the user space.
    UserSyscall,
    /// A CPU exception is triggered by the user space.
    UserException,
    /// A kernel event is pending
    KernelEvent,
}

/// Hooks that will be called during [`UserMode::execute`].
pub trait UserModeHooks {
    /// Checks whether a kernel event is pending.
    ///
    /// This method will be called after user space is interrupted
    /// by external interrupts. If the result is `true`,
    /// [`UserMode::execute`] will return with [`ReturnReason::KernelEvent`].
    fn has_kernel_event(&self) -> bool {
        false
    }

    /// Prepares user space execution.
    ///
    /// This method will be called just before entering user space.
    /// Local IRQs are disabled and will only be enabled after entering user space.
    fn pre_user_run(&self, _guard: &DisabledLocalIrqGuard) {}
}

/// A struct that provides dummy (no-op) user mode hooks.
pub struct DummyUserHooks;

impl UserModeHooks for DummyUserHooks {}
