use core::sync::atomic::AtomicUsize;

use jinux_frame::{
    cpu::CpuContext,
    task::Task,
    user::{UserEvent, UserMode, UserSpace},
};

use crate::{
    prelude::*,
    process::{exception::handle_exception, signal::handle_pending_signal},
    rights::Full,
    vm::vmar::Vmar,
};

use crate::syscall::handle_syscall;

use super::{elf::load_elf_to_root_vmar, Process};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn create_user_task_from_elf(
    root_vmar: &Vmar<Full>,
    filename: CString,
    elf_file_content: &'static [u8],
    parent: Weak<Process>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Arc<Task> {
    let elf_load_info = load_elf_to_root_vmar(filename, elf_file_content, &root_vmar, argv, envp)
        .expect("Load Elf failed");
    let vm_space = root_vmar.vm_space().clone();
    let mut cpu_ctx = CpuContext::default();
    // set entry point
    cpu_ctx.gp_regs.rip = elf_load_info.entry_point();
    // set user stack
    cpu_ctx.gp_regs.rsp = elf_load_info.user_stack_top();
    let user_space = Arc::new(UserSpace::new(vm_space, cpu_ctx));
    create_new_task(user_space, parent)
}

/// create new task with userspace and parent process
pub fn create_new_task(userspace: Arc<UserSpace>, parent: Weak<Process>) -> Arc<Task> {
    fn user_task_entry() {
        let cur = Task::current();
        let user_space = cur.user_space().expect("user task should have user space");
        let mut user_mode = UserMode::new(user_space);
        debug!("In user task entry:");
        debug!("[new task] rip = 0x{:x}", user_space.cpu_ctx.gp_regs.rip);
        debug!("[new task] rsp = 0x{:x}", user_space.cpu_ctx.gp_regs.rsp);
        debug!("[new task] rax = 0x{:x}", user_space.cpu_ctx.gp_regs.rax);
        loop {
            let user_event = user_mode.execute();
            let context = user_mode.context_mut();
            // handle user event:
            handle_user_event(user_event, context);
            let current = current!();
            // should be do this comparison before handle signal?
            if current.status().lock().is_zombie() {
                break;
            }
            handle_pending_signal(context).unwrap();
            if current.status().lock().is_zombie() {
                debug!("exit due to signal");
                break;
            }
            // If current is suspended, wait for a signal to wake up self
            while current.status().lock().is_suspend() {
                Process::yield_now();
                debug!("{} is suspended.", current.pid());
                handle_pending_signal(context).unwrap();
            }
        }
        debug!("exit user loop");
        // FIXME: This is a work around: exit in kernel task entry may be not called. Why this will happen?
        Task::current().exit();
    }

    Task::new(user_task_entry, parent, Some(userspace)).expect("spawn task failed")
}

fn handle_user_event(user_event: UserEvent, context: &mut CpuContext) {
    match user_event {
        UserEvent::Syscall => handle_syscall(context),
        UserEvent::Fault => todo!(),
        UserEvent::Exception => handle_exception(context),
    }
}
