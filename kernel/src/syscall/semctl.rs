// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    ipc::{
        IpcControlCmd, IpcNamespace,
        semaphore::system_v::{PermissionMode, sem::Semaphore, sem_set::SemaphoreSet},
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
        return_errno!(Errno::EINVAL)
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
            let mut sem_sets_mut = ipc_ns.sem_sets_mut();
            let sem_set = sem_sets_mut.get(&semid).ok_or(Error::new(Errno::EINVAL))?;

            let euid = ctx.posix_thread.credentials().euid();
            let permission = sem_set.permission();
            let can_remove = (euid == permission.uid()) || (euid == permission.cuid());
            if !can_remove {
                return_errno!(Errno::EPERM);
            }

            sem_sets_mut
                .remove(&semid)
                .ok_or(Error::new(Errno::EINVAL))?;

            // Free the ID after releasing the write lock to maintain
            // a consistent lock ordering with `create_sem_set`.
            drop(sem_sets_mut);
            ipc_ns.free_sem_id(semid);
        }
        IpcControlCmd::SEM_SETVAL => {
            // In setval, arg is parse as i32
            let val = arg as i32;
            if val < 0 {
                return_errno!(Errno::ERANGE);
            }

            check_and_ctl(ipc_ns, semid, PermissionMode::ALTER, |sem_set| {
                sem_set.setval(semnum as usize, val, ctx.process.pid())
            })?;
        }
        IpcControlCmd::SEM_GETVAL => {
            fn sem_val(sem: &Semaphore) -> i32 {
                sem.val()
            }
            let val: i32 = check_and_ctl(ipc_ns, semid, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, &sem_val)
            })?;

            return Ok(SyscallReturn::Return(val as isize));
        }
        IpcControlCmd::SEM_GETPID => {
            fn sem_pid(sem: &Semaphore) -> Pid {
                sem.latest_modified_pid()
            }
            let pid: Pid = check_and_ctl(ipc_ns, semid, PermissionMode::READ, |sem_set| {
                sem_set.get(semnum as usize, &sem_pid)
            })?;

            return Ok(SyscallReturn::Return(pid as isize));
        }
        IpcControlCmd::SEM_GETZCNT => {
            let cnt: usize = check_and_ctl(ipc_ns, semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set.pending_const_count(semnum as u16))
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::SEM_GETNCNT => {
            let cnt: usize = check_and_ctl(ipc_ns, semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set.pending_alter_count(semnum as u16))
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::IPC_STAT => {
            check_and_ctl(ipc_ns, semid, PermissionMode::READ, |sem_set| {
                let semid_ds = sem_set.semid_ds();
                Ok(ctx.user_space().write_val(arg as Vaddr, &semid_ds)?)
            })?;
        }
        _ => todo!("Need to support {:?} in SYS_SEMCTL", cmd),
    }

    Ok(SyscallReturn::Return(0))
}

fn check_and_ctl<T, F>(
    ipc_ns: &IpcNamespace,
    semid: i32,
    permission: PermissionMode,
    ctl_func: F,
) -> Result<T>
where
    F: FnOnce(&SemaphoreSet) -> Result<T>,
{
    let sem_sets = ipc_ns.sem_sets();
    let sem_set = sem_sets.get(&semid).ok_or(Error::new(Errno::EINVAL))?;

    if !permission.is_empty() {
        // TODO: Support permission check.
        debug!("Semaphore doesn't support permission check now");
    }

    ctl_func(sem_set)
}
