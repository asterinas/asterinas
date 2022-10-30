use core::sync::atomic::AtomicUsize;

use kxos_frame::{
    cpu::CpuContext,
    task::Task,
    user::{UserEvent, UserMode, UserSpace},
    vm::VmSpace,
};

use crate::prelude::*;

use crate::syscall::syscall_handler;

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
            if let HandlerResult::Exit = handle_user_event(user_event, context) {
                // FIXME: How to set task status? How to set exit code of process?
                break;
            }
            // debug!("before return to user space: {:#x?}", context);
        }
        let current_process = Process::current();
        current_process.exit();
    }

    Task::new(user_task_entry, parent, Some(userspace)).expect("spawn task failed")
}

fn handle_user_event(user_event: UserEvent, context: &mut CpuContext) -> HandlerResult {
    match user_event {
        UserEvent::Syscall => syscall_handler(context),
        UserEvent::Fault => todo!(),
        UserEvent::Exception => todo!(),
    }
}

pub enum HandlerResult {
    Exit,
    Continue,
}
