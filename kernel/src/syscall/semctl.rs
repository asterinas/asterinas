// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    ipc::{
        IpcControlCmd,
        semaphore::system_v::{PermissionMode, sem::Semaphore},
    },
    prelude::*,
    process::Pid,
};

pub fn sys_semctl(
    semid: i32,
    semnum: i32,
    cmd: i32,
    arg: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if semid <= 0 || semnum < 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid semid or semnum")
    }

    let cmd = IpcControlCmd::try_from(cmd)?;
    debug!(
        "[sys_semctl] semid = {}, semnum = {}, cmd = {:?}, arg = {:x}",
        semid, semnum, cmd, arg
    );

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let ipc_ns = ns_proxy.unwrap().ipc_ns();

    match cmd {
        IpcControlCmd::IPC_RMID => {
            let euid = ctx.posix_thread.credentials().euid();
            ipc_ns.remove_sem_set(semid, |sem_set| {
                let permission = sem_set.permission();
                let can_remove = (euid == permission.uid()) || (euid == permission.cuid());
                if !can_remove {
                    return_errno_with_message!(
                        Errno::EPERM,
                        "no permission to remove semaphore set"
                    );
                }

                Ok(())
            })?;
        }
        IpcControlCmd::SEM_SETVAL => {
            // In setval, arg is parse as i32
            let val = arg as i32;
            if val < 0 {
                return_errno_with_message!(Errno::ERANGE, "semaphore value must not be negative");
            }

            ipc_ns.with_sem_set(semid, None, PermissionMode::ALTER, |sem_set| {
                sem_set.setval(semnum as usize, val, ctx.process.pid())
            })?;
        }
        IpcControlCmd::SEM_GETVAL => {
            fn sem_val(sem: &Semaphore) -> i32 {
                sem.val()
            }
            let val: i32 = ipc_ns.with_sem_set(semid, None, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, &sem_val)
            })?;

            return Ok(SyscallReturn::Return(val as isize));
        }
        IpcControlCmd::SEM_GETPID => {
            fn sem_pid(sem: &Semaphore) -> Pid {
                sem.latest_modified_pid()
            }
            let pid: Pid = ipc_ns.with_sem_set(semid, None, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, &sem_pid)
            })?;

            return Ok(SyscallReturn::Return(pid as isize));
        }
        IpcControlCmd::SEM_GETZCNT => {
            let cnt: usize = ipc_ns.with_sem_set(semid, None, PermissionMode::READ, |sem_set| {
                Ok(sem_set.pending_const_count(semnum as u16))
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::SEM_GETNCNT => {
            let cnt: usize = ipc_ns.with_sem_set(semid, None, PermissionMode::READ, |sem_set| {
                Ok(sem_set.pending_alter_count(semnum as u16))
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::IPC_STAT => {
            ipc_ns.with_sem_set(semid, None, PermissionMode::READ, |sem_set| {
                let semid_ds = sem_set.semid_ds();
                Ok(ctx.user_space().write_val(arg as Vaddr, &semid_ds)?)
            })?;
        }
        _ => todo!("Need to support {:?} in SYS_SEMCTL", cmd),
    }

    Ok(SyscallReturn::Return(0))
}
