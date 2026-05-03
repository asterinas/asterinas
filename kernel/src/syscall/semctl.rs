// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    ipc::{
        IpcControlCmd, IpcId,
        semaphore::system_v::{PermissionMode, sem::Semaphore},
    },
    prelude::*,
    process::Pid,
};

pub fn sys_semctl(
    semid: i32,
    semnum: i32,
    op: i32,
    arg: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let Ok(semid) = IpcId::try_from(semid.cast_unsigned()) else {
        return_errno_with_message!(Errno::EINVAL, "non-positive semaphore IDs are invalid");
    };
    let cmd = IpcControlCmd::try_from(op)?;

    debug!(
        "semctl: semid = {:?}, semnum = {}, cmd = {:?}, arg = {:x}",
        semid, semnum, cmd, arg
    );

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let ipc_ns = ns_proxy.unwrap().ipc_ns();

    match cmd {
        IpcControlCmd::IPC_RMID => {
            let euid = ctx.posix_thread.credentials().euid();
            ipc_ns.remove_sem_set(semid, |sem_set| {
                // TODO: Consider capabilities in addition to UIDs.
                let permission = sem_set.permission();
                let can_remove = (euid == permission.uid()) || (euid == permission.cuid());
                if !can_remove {
                    return_errno_with_message!(
                        Errno::EPERM,
                        "the process does not have permission to remove the semaphore set"
                    );
                }

                Ok(())
            })?;
        }
        IpcControlCmd::IPC_STAT => {
            ipc_ns.with_sem_set(semid, PermissionMode::READ, |sem_set| {
                let semid_ds = sem_set.semid_ds();
                Ok(ctx.user_space().write_val(arg as Vaddr, &semid_ds)?)
            })?;
        }
        IpcControlCmd::SEM_GETPID => {
            fn sem_pid(sem: &Semaphore) -> Pid {
                sem.latest_modified_pid()
            }
            let pid: Pid = ipc_ns.with_sem_set(semid, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, sem_pid)
            })?;

            return Ok(SyscallReturn::Return(pid as isize));
        }
        IpcControlCmd::SEM_GETVAL => {
            fn sem_val(sem: &Semaphore) -> i32 {
                sem.val()
            }
            let val: i32 = ipc_ns.with_sem_set(semid, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, sem_val)
            })?;

            return Ok(SyscallReturn::Return(val as isize));
        }
        IpcControlCmd::SEM_GETNCNT => {
            let cnt: usize = ipc_ns.with_sem_set(semid, PermissionMode::READ, |sem_set| {
                sem_set.pending_alter_count(semnum as usize)
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::SEM_GETZCNT => {
            let cnt: usize = ipc_ns.with_sem_set(semid, PermissionMode::READ, |sem_set| {
                sem_set.pending_const_count(semnum as usize)
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::SEM_SETVAL => {
            // In `SEM_SETVAL`, the argument is parsed as an `i32`.
            let val = arg as i32;

            ipc_ns.with_sem_set(semid, PermissionMode::ALTER, |sem_set| {
                sem_set.setval(semnum as usize, val, ctx.process.pid())
            })?;
        }
    }

    Ok(SyscallReturn::Return(0))
}
