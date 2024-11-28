// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! User space.

use crate::{
    cpu::{FpuState, UserContext},
    mm::VmSpace,
    prelude::*,
    trap::TrapFrame,
};

/// A user space.
///
/// Each user space has a VM address space and allows a task to execute in
/// user mode.
#[derive(Debug)]
pub struct UserSpace {
    /// vm space
    vm_space: Arc<VmSpace>,
    /// cpu context before entering user space
    init_ctx: UserContext,
}

impl UserSpace {
    /// Creates a new instance.
    ///
    /// Each instance maintains a VM address space and the CPU state to enable
    /// execution in the user space.
    pub fn new(vm_space: Arc<VmSpace>, init_ctx: UserContext) -> Self {
        Self { vm_space, init_ctx }
    }

    /// Returns the VM address space.
    pub fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    /// Returns the user mode that is bound to the current task and user space.
    ///
    /// See [`UserMode`] on how to use it to execute user code.
    ///
    /// # Panics
    ///
    /// This method is intended to only allow each task to have at most one
    /// instance of [`UserMode`] initiated. If this method is called again before
    /// the first instance for the current task is dropped, then the method
    /// panics.      
    pub fn user_mode(&self) -> UserMode<'_> {
        todo!()
    }

    /// Sets thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.init_ctx.set_tls_pointer(tls)
    }

    /// Gets thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.init_ctx.tls_pointer()
    }

    /// Gets a reference to the FPU state.
    pub fn fpu_state(&self) -> &FpuState {
        self.init_ctx.fpu_state()
    }
}

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
/// let user_space = current.user_space()
///     .expect("the current task is not associated with a user space");
/// let mut user_mode = user_space.user_mode();
/// loop {
///     // Execute in the user space until some interesting events occur.
///     let return_reason = user_mode.execute(|| false);
///     todo!("handle the event, e.g., syscall");
/// }
/// ```
pub struct UserMode<'a> {
    user_space: &'a Arc<UserSpace>,
    context: UserContext,
}

// An instance of `UserMode` is bound to the current task. So it must not be sent to other tasks.
impl !Send for UserMode<'_> {}
// Note that implementing `!Sync` is unnecessary
// because entering the user space via `UserMode` requires taking a mutable reference.

impl<'a> UserMode<'a> {
    /// Creates a new `UserMode`.
    pub fn new(user_space: &'a Arc<UserSpace>) -> Self {
        Self {
            user_space,
            context: user_space.init_ctx.clone(),
        }
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

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
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
