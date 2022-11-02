use crate::{
    prelude::*,
    process::clone::{clone_child, CloneArgs},
};
use kxos_frame::cpu::CpuContext;

use crate::{process::Process, syscall::SYS_FORK};

pub fn sys_fork(parent_context: CpuContext) -> Result<isize> {
    debug!("[syscall][id={}][SYS_FORK]", SYS_FORK);
    let child_process = fork(parent_context);
    Ok(child_process.pid() as _)
}

/// Fork a child process
fn fork(parent_context: CpuContext) -> Arc<Process> {
    let current = current!();
    // FIXME: set correct args for fork
    let clone_args = CloneArgs::default();
    let child = clone_child(parent_context, clone_args).unwrap();
    let pid = current.pid();
    debug!("*********schedule child process, pid = {}**********", pid);
    child.send_to_scheduler();
    debug!("*********return to parent process, pid = {}*********", pid);
    child
}
