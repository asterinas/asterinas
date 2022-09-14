use core::sync::atomic::AtomicUsize;

use alloc::sync::{Arc, Weak};
use kxos_frame::{
    cpu::CpuContext,
    debug,
    task::Task,
    user::{UserEvent, UserMode, UserSpace},
    vm::VmSpace,
};

use crate::{
    memory::load_elf_to_vm_space,
    process::{current_pid, current_process},
    syscall::syscall_handler,
};

use super::Process;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn create_user_task_from_elf(elf_file_content: &[u8], process: Weak<Process>) -> Arc<Task> {
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
            debug!("current pid = {}", current_pid());
            let context = user_mode.context_mut();
            if let HandlerResult::Exit = handle_user_event(user_event, context) {
                // FIXME: How to set task status? How to set exit code of process?
                break;
            }
        }
        let current_process = current_process();
        // Work Around: We schedule all child tasks to run when current process exit.
        if current_process.has_child() {
            debug!("*********schedule child process**********");
            let child_process = current_process.get_child_process();
            child_process.send_to_scheduler();
            debug!("*********return to parent process*********");
        }
        // exit current process
        current_process.exit();
    }

    Task::new(user_task_entry, process, Some(user_space)).expect("spawn user task failed.")
}

pub fn create_forked_task(userspace: Arc<UserSpace>, process: Weak<Process>) -> Arc<Task> {
    fn user_task_entry() {
        let cur = Task::current();
        let user_space = cur.user_space().expect("user task should have user space");
        let mut user_mode = UserMode::new(user_space);
        debug!("In forked task");
        debug!("[forked task] pid = {}", current_pid());
        debug!("[forked task] rip = 0x{:x}", user_space.cpu_ctx.gp_regs.rip);
        debug!("[forked task] rsp = 0x{:x}", user_space.cpu_ctx.gp_regs.rsp);
        debug!("[forked task] rax = 0x{:x}", user_space.cpu_ctx.gp_regs.rax);
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

    Task::new(user_task_entry, process, Some(userspace)).expect("spawn task failed")
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
