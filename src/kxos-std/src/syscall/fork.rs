use alloc::{sync::Arc, vec};
use kxos_frame::{
    cpu::CpuContext,
    debug,
    user::UserSpace,
    vm::{VmIo, VmSpace},
};

use crate::{
    process::{new_pid, task::create_new_task, Process},
    syscall::SYS_FORK,
};

use super::SyscallResult;

pub fn sys_fork(parent_context: CpuContext) -> SyscallResult {
    debug!("[syscall][id={}][SYS_FORK]", SYS_FORK);
    let child_process = fork(parent_context);
    SyscallResult::Return(child_process.pid() as i32)
}

/// Fork a child process
/// WorkAround: This function only create a new process, but did not schedule the process to run
fn fork(parent_context: CpuContext) -> Arc<Process> {
    let child_pid = new_pid();
    let current = Process::current();

    // child process vm space
    // FIXME: COPY ON WRITE can be used here
    let parent_vm_space = current
        .vm_space()
        .expect("User task should always have vm space");
    let child_vm_space = parent_vm_space.clone();
    debug_check_fork_vm_space(parent_vm_space, &child_vm_space);

    let child_file_name = match current.filename() {
        None => None,
        Some(filename) => Some(filename.clone()),
    };

    // child process user_vm
    let child_user_vm = match current.user_vm() {
        None => None,
        Some(user_vm) => Some(user_vm.clone()),
    };

    // child process cpu context
    let mut child_cpu_context = parent_context.clone();
    debug!("parent cpu context: {:?}", child_cpu_context.gp_regs);
    child_cpu_context.gp_regs.rax = 0; // Set return value of child process

    let child_user_space = Arc::new(UserSpace::new(child_vm_space, child_cpu_context));
    debug!("before spawn child task");
    debug!("current pid: {}", current.pid());
    debug!("child process pid: {}", child_pid);
    debug!("rip = 0x{:x}", child_cpu_context.gp_regs.rip);

    let child = Arc::new_cyclic(|child_process_ref| {
        let weak_child_process = child_process_ref.clone();
        let child_task = create_new_task(child_user_space.clone(), weak_child_process);
        Process::new(
            child_pid,
            child_task,
            child_file_name,
            child_user_vm,
            Some(child_user_space),
        )
    });
    Process::current().add_child(child.clone());
    let pid = current.pid();
    debug!("*********schedule child process, pid = {}**********", pid);
    child.send_to_scheduler();
    debug!("*********return to parent process, pid = {}*********", pid);
    child
}

/// debug use
fn debug_check_fork_vm_space(parent_vm_space: &VmSpace, child_vm_space: &VmSpace) {
    let mut buffer1 = vec![0u8; 0x78];
    let mut buffer2 = vec![0u8; 0x78];
    parent_vm_space
        .read_bytes(0x401000, &mut buffer1)
        .expect("read buffer1 failed");
    child_vm_space
        .read_bytes(0x401000, &mut buffer2)
        .expect("read buffer1 failed");
    for len in 0..buffer1.len() {
        assert_eq!(buffer1[len], buffer2[len]);
    }
    debug!("check fork vm space succeed.");
}
