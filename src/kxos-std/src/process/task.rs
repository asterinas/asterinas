use alloc::sync::Arc;
use kxos_frame::{
    cpu::CpuContext,
    debug,
    task::Task,
    user::{UserEvent, UserMode, UserSpace},
    vm::VmSpace,
};

use crate::{memory::load_elf_to_vm_space, syscall::syscall_handler};

pub fn spawn_user_task_from_elf(elf_file_content: &[u8]) -> Arc<Task> {
    let vm_space = VmSpace::new();
    let elf_load_info = load_elf_to_vm_space(elf_file_content, &vm_space).expect("Load Elf failed");
    let mut cpu_ctx = CpuContext::default();
    // FIXME: correct regs?
    // set entry point
    cpu_ctx.gp_regs.rip = elf_load_info.entry_point();
    // set user stack
    cpu_ctx.gp_regs.rsp = elf_load_info.user_stack_top();

    let user_space = Arc::new(UserSpace::new(vm_space, cpu_ctx));

    fn user_task_entry() {
        let cur = Task::current();
        let user_space = cur.user_space().expect("user task should have user space");
        let mut user_mode = UserMode::new(user_space);
        loop {
            let user_event = user_mode.execute();
            debug!("return from user mode");
            let context = user_mode.context_mut();
            if let HandlerResult::Exit = handle_user_event(user_event, context) {
                // FIXME: How to set task status? How to set exit code of process?
                break;
            }
        }
    }

    // FIXME: set the correct type when task has no data
    Task::spawn(user_task_entry, None::<u8>, Some(user_space)).expect("spawn user task failed.")
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
