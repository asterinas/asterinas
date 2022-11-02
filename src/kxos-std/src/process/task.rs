use core::sync::atomic::AtomicUsize;

use kxos_frame::{
    cpu::CpuContext,
    task::Task,
    user::{UserEvent, UserMode, UserSpace},
    vm::VmSpace,
};

use crate::{
    prelude::*,
    process::{exception::handle_exception, signal::handle_pending_signal},
};

use crate::syscall::handle_syscall;

use super::{elf::load_elf_to_vm_space, Process};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn create_user_task_from_elf(
    filename: CString,
    elf_file_content: &[u8],
    parent: Weak<Process>,
) -> Arc<Task> {
    let vm_space = VmSpace::new();
    let elf_load_info =
        load_elf_to_vm_space(filename, elf_file_content, &vm_space).expect("Load Elf failed");
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
        debug!("In new task");
        debug!("[new task] pid = {}", Process::current().pid());
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
            handle_pending_signal();
            if current.status().lock().is_zombie() {
                debug!("exit due to signal");
                break;
            }
            // If current is suspended, wait for a signal to wake up self
            while current.status().lock().is_suspend() {
                Process::yield_now();
                debug!("{} is suspended.", current.pid());
                handle_pending_signal();
            }
        }
        debug!("exit user loop");
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
