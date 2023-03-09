//! User space.

use crate::trap::call_irq_callback_functions;
use crate::x86_64_util::{self, rdfsbase, wrfsbase};
use log::debug;
use trapframe::{TrapFrame, UserContext};
use x86_64::registers::rflags::RFlags;

use crate::cpu::CpuContext;
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
    user_space: &'a Arc<UserSpace>,
    context: CpuContext,
    user_context: UserContext,
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
            user_context: UserContext::default(),
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
            self.context = self.user_space.cpu_ctx;
            if self.context.gp_regs.rflag == 0 {
                self.context.gp_regs.rflag = (RFlags::INTERRUPT_FLAG | RFlags::ID).bits() | 0x2;
            }
            // write fsbase
            wrfsbase(self.user_space.cpu_ctx.fs_base);
            let fp_regs = self.user_space.cpu_ctx.fp_regs;
            if fp_regs.is_valid() {
                fp_regs.restore();
            }
            self.executed = true;
        } else {
            // write fsbase
            if rdfsbase() != self.context.fs_base {
                debug!("write fsbase: 0x{:x}", self.context.fs_base);
                wrfsbase(self.context.fs_base);
            }

            // write fp_regs
            // let fp_regs = self.context.fp_regs;
            // if fp_regs.is_valid() {
            //     fp_regs.restore();
            // }
        }
        self.user_context = self.context.into();
        self.user_context.run();
        let mut trap_frame;
        while self.user_context.trap_num >= 0x20 && self.user_context.trap_num < 0x100 {
            trap_frame = TrapFrame {
                rax: self.user_context.general.rax,
                rbx: self.user_context.general.rbx,
                rcx: self.user_context.general.rcx,
                rdx: self.user_context.general.rdx,
                rsi: self.user_context.general.rsi,
                rdi: self.user_context.general.rdi,
                rbp: self.user_context.general.rbp,
                rsp: self.user_context.general.rsp,
                r8: self.user_context.general.r8,
                r9: self.user_context.general.r9,
                r10: self.user_context.general.r10,
                r11: self.user_context.general.r11,
                r12: self.user_context.general.r12,
                r13: self.user_context.general.r13,
                r14: self.user_context.general.r14,
                r15: self.user_context.general.r15,
                _pad: 0,
                trap_num: self.user_context.trap_num,
                error_code: self.user_context.error_code,
                rip: self.user_context.general.rip,
                cs: 0,
                rflags: self.user_context.general.rflags,
            };
            call_irq_callback_functions(&mut trap_frame);
            self.user_context.run();
        }
        x86_64::instructions::interrupts::enable();
        self.context = CpuContext::from(self.user_context);
        if self.user_context.trap_num != 0x100 {
            self.context.fs_base = rdfsbase();
            // self.context.fp_regs.save();
            UserEvent::Exception
        } else {
            self.context.fs_base = rdfsbase();
            // self.context.fp_regs.save();
            // debug!("[kernel] syscall id:{}", self.context.gp_regs.rax);
            // debug!("[kernel] rsp: 0x{:x}", self.context.gp_regs.rsp);
            // debug!("[kernel] rcx: 0x{:x}", self.context.gp_regs.rcx);
            // debug!("[kernel] rip: 0x{:x}", self.context.gp_regs.rip);
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
