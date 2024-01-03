// SPDX-License-Identifier: MPL-2.0

//! User space.

use crate::cpu::UserContext;
use crate::prelude::*;
use crate::task::Task;
use crate::vm::VmSpace;
use trapframe::TrapFrame;

/// A user space.
///
/// Each user space has a VM address space and allows a task to execute in
/// user mode.
pub struct UserSpace {
    /// vm space
    vm_space: VmSpace,
    /// cpu context before entering user space
    init_ctx: UserContext,
}

impl UserSpace {
    /// Creates a new instance.
    ///
    /// Each instance maintains a VM address space and the CPU state to enable
    /// execution in the user space.
    pub fn new(vm_space: VmSpace, init_ctx: UserContext) -> Self {
        Self { vm_space, init_ctx }
    }

    /// Returns the VM address space.
    pub fn vm_space(&self) -> &VmSpace {
        &self.vm_space
    }

    /// Returns the user mode that is bound to the current task and user space.
    ///
    /// See `UserMode` on how to use it to execute user code.
    ///
    /// # Panic
    ///
    /// This method is intended to only allow each task to have at most one
    /// instance of `UserMode` initiated. If this method is called again before
    /// the first instance for the current task is dropped, then the method
    /// panics.      
    pub fn user_mode(&self) -> UserMode<'_> {
        todo!()
    }
}

/// Specific architectures need to implement this trait. This should only used in `UserMode`
///
/// Only visible in aster-frame
pub(crate) trait UserContextApiInternal {
    /// Starts executing in the user mode.
    fn execute(&mut self) -> UserEvent;

    /// Use the information inside CpuContext to build a trapframe
    fn as_trap_frame(&self) -> TrapFrame;
}

/// The common interface that every CPU architecture-specific `CpuContext` implements.
pub trait UserContextApi {
    /// Get the trap number of this interrupt.
    fn trap_number(&self) -> usize;

    /// Get the trap error code of this interrupt.
    fn trap_error_code(&self) -> usize;

    /// Get number of syscall
    fn syscall_num(&self) -> usize;

    /// Get return value of syscall
    fn syscall_ret(&self) -> usize;

    /// Set return value of syscall
    fn set_syscall_ret(&mut self, ret: usize);

    /// Get syscall args
    fn syscall_args(&self) -> [usize; 6];

    /// Set instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Get instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Set stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Get stack pointer
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
/// use aster_frame::task::Task;
///
/// let current = Task::current();
/// let user_space = current.user_space()
///     .expect("the current task is associated with a user space");
/// let mut user_mode = user_space.user_mode();
/// loop {
///     // Execute in the user space until some interesting user event occurs
///     let user_event = user_mode.execute();
///     todo!("handle the user event, e.g., syscall");
/// }
/// ```
pub struct UserMode<'a> {
    current: Arc<Task>,
    user_space: &'a Arc<UserSpace>,
    context: UserContext,
}

// An instance of `UserMode` is bound to the current task. So it cannot be
impl<'a> !Send for UserMode<'a> {}

impl<'a> UserMode<'a> {
    pub fn new(user_space: &'a Arc<UserSpace>) -> Self {
        Self {
            current: Task::current(),
            user_space,
            context: user_space.init_ctx,
        }
    }

    /// Starts executing in the user mode. Make sure current task is the task in `UserMode`.
    ///
    /// The method returns for one of three possible reasons indicated by `UserEvent`.
    /// 1. The user invokes a system call;
    /// 2. The user triggers an exception;
    /// 3. The user triggers a fault.
    ///
    /// After handling the user event and updating the user-mode CPU context,
    /// this method can be invoked again to go back to the user space.
    pub fn execute(&mut self) -> UserEvent {
        unsafe {
            self.user_space.vm_space().activate();
        }
        debug_assert!(Arc::ptr_eq(&self.current, &Task::current()));
        self.context.execute()
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
/// A user event is what brings back the control of the CPU back from
/// the user space to the kernel space.
///
/// Note that hardware interrupts are not considered user events as they
/// are triggered by devices and not visible to user programs.
/// To handle interrupts, one should register callback funtions for
/// IRQ lines (`IrqLine`).
pub enum UserEvent {
    Syscall,
    Exception,
}
