//! User space.

use trapframe::UserContext;

cfg_if::cfg_if! {
    if #[cfg(feature="x86_64")]{
        use crate::arch::x86::cpu::CpuContext;
    }
}

use crate::prelude::*;
use crate::task::Task;
use crate::vm::VmSpace;

/// A user space.
///
/// Each user space has a VM address space and allows a task to execute in
/// user mode.
pub struct UserSpace {
    /// vm space
    vm_space: VmSpace,
    /// cpu context before entering user space
    pub cpu_ctx: CpuContext,
}

impl UserSpace {
    /// Creates a new instance.
    ///
    /// Each instance maintains a VM address space and the CPU state to enable
    /// execution in the user space.
    pub fn new(vm_space: VmSpace, cpu_ctx: CpuContext) -> Self {
        Self {
            vm_space: vm_space,
            cpu_ctx: cpu_ctx,
        }
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

/// Specific architectures need to implement this trait so that UserMode tasks can be executed
pub trait UserModeExecute {
    /// Starts executing in the user mode. Make sure current task is the task in `UserMode`.
    ///
    /// The method returns for one of three possible reasons indicated by `UserEvent`.
    /// 1. The user invokes a system call;
    /// 2. The user triggers an exception;
    /// 3. The user triggers a fault.
    ///
    /// After handling the user event and updating the user-mode CPU context,
    /// this method can be invoked again to go back to the user space.
    fn execute(&mut self) -> UserEvent;
}

/// Code execution in the user mode.
///
/// This type enables executing the code in user space from a task in the kernel
/// space safely.
///
/// Here is a sample code on how to use `UserMode`.
///  
/// ```no_run
/// use jinux_frame::task::Task;
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
    pub(crate) user_space: &'a Arc<UserSpace>,
    pub(crate) context: CpuContext,
    pub(crate) user_context: UserContext,
    pub(crate) executed: bool,
}

// An instance of `UserMode` is bound to the current task. So it cannot be
impl<'a> !Send for UserMode<'a> {}

impl<'a> UserMode<'a> {
    pub fn new(user_space: &'a Arc<UserSpace>) -> Self {
        Self {
            current: Task::current(),
            user_space,
            context: CpuContext::default(),
            executed: false,
            user_context: UserContext::default(),
        }
    }

    /// Returns an immutable reference the user-mode CPU context.
    pub fn context(&self) -> &CpuContext {
        &self.context
    }

    /// Returns a mutable reference the user-mode CPU context.
    pub fn context_mut(&mut self) -> &mut CpuContext {
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
    Fault,
}
