//! User space.

use crate::println;

use crate::cpu::CpuContext;
use crate::prelude::*;
use crate::task::{context_switch, Task, TaskContext, SWITCH_TO_USER_SPACE_TASK};
use crate::trap::{SyscallFrame, TrapFrame};
use crate::vm::VmSpace;

extern "C" {
    pub(crate) fn syscall_switch_to_user_space(
        cpu_context: &CpuContext,
        syscall_frame: &SyscallFrame,
    );
    /// cpu_context may delete in the future
    pub(crate) fn trap_switch_to_user_space(cpu_context: &CpuContext, trap_frame: &TrapFrame);
}

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

/// Code execution in the user mode.
///
/// This type enables executing the code in user space from a task in the kernel
/// space safely.
///
/// Here is a sample code on how to use `UserMode`.
///  
/// ```no_run
/// use kxos_frame::task::Task;
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
    context: CpuContext,
    executed: bool,
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
        if !self.executed {
            self.current.syscall_frame().caller.rcx = self.user_space.cpu_ctx.gp_regs.rip as usize;
            self.current.syscall_frame().callee.rsp = self.user_space.cpu_ctx.gp_regs.rsp as usize;
            self.current.syscall_frame().caller.rax = self.user_space.cpu_ctx.gp_regs.rax as usize;
            self.executed = true;
        } else {
            if self.current.inner_exclusive_access().is_from_trap {
                *self.current.trap_frame() = self.context.into();
            } else {
                *self.current.syscall_frame() = self.context.into();
            }
        }
        let mut current_task_inner = self.current.inner_exclusive_access();
        let binding = SWITCH_TO_USER_SPACE_TASK.get();
        let next_task_inner = binding.inner_exclusive_access();
        let current_ctx = &mut current_task_inner.ctx as *mut TaskContext;
        let next_ctx = &next_task_inner.ctx as *const TaskContext;
        drop(current_task_inner);
        drop(next_task_inner);
        drop(binding);
        unsafe {
            context_switch(current_ctx, next_ctx);
            // switch_to_user_space(&self.user_space.cpu_ctx, self.current.syscall_frame());
        }
        if self.current.inner_exclusive_access().is_from_trap {
            self.context = CpuContext::from(*self.current.trap_frame());
            UserEvent::Exception
        } else {
            self.context = CpuContext::from(*self.current.syscall_frame());
            debug!("[kernel] syscall id:{}", self.context.gp_regs.rax);
            debug!("[kernel] rsp: 0x{:x}", self.context.gp_regs.rsp);
            UserEvent::Syscall
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
