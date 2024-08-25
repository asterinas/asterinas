// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{
        semaphore::system_v::{
            sem_set::{check_sem, sem_sets, sem_sets_mut, SemaphoreSet},
            PermissionMode,
        },
        IpcControlCmd,
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

    match cmd {
        IpcControlCmd::IPC_RMID => {
            let mut sem_sets_mut = sem_sets_mut();
            let sem_set = sem_sets_mut.get(&semid).ok_or(Error::new(Errno::EINVAL))?;

            let euid = ctx.posix_thread.credentials().euid();
            let permission = sem_set.permission();
            let can_removed = (euid == permission.uid()) || (euid == permission.cuid());
            if !can_removed {
                return_errno!(Errno::EPERM);
            }

            sem_sets_mut
                .remove(&semid)
                .ok_or(Error::new(Errno::EINVAL))?;
        }
        IpcControlCmd::SEM_SETVAL => {
            // In setval, arg is parse as i32
            let val = arg as i32;
            if val < 0 {
                return_errno!(Errno::ERANGE);
            }

            check_and_ctl(semid, PermissionMode::ALTER, |sem_set| {
                let sem = sem_set
                    .get(semnum as usize)
                    .ok_or(Error::new(Errno::EINVAL))?;

                sem.set_val(val, ctx.process.pid())?;
                sem_set.update_ctime();

                Ok(())
            })?;
        }
        IpcControlCmd::SEM_GETVAL => {
            let val: i32 = check_and_ctl(semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set
                    .get(semnum as usize)
                    .ok_or(Error::new(Errno::EINVAL))?
                    .val())
            })?;

            return Ok(SyscallReturn::Return(val as isize));
        }
        IpcControlCmd::SEM_GETPID => {
            let pid: Pid = check_and_ctl(semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set
                    .get(semnum as usize)
                    .ok_or(Error::new(Errno::EINVAL))?
                    .last_modified_pid())
            })?;

            return Ok(SyscallReturn::Return(pid as isize));
        }
        IpcControlCmd::SEM_GETZCNT => {
            let cnt: usize = check_and_ctl(semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set
                    .get(semnum as usize)
                    .ok_or(Error::new(Errno::EINVAL))?
                    .pending_zero_count())
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        IpcControlCmd::SEM_GETNCNT => {
            let cnt: usize = check_and_ctl(semid, PermissionMode::READ, |sem_set| {
                Ok(sem_set
                    .get(semnum as usize)
                    .ok_or(Error::new(Errno::EINVAL))?
                    .pending_alter_count())
            })?;

            return Ok(SyscallReturn::Return(cnt as isize));
        }
        _ => todo!("Need to support {:?} in SYS_SEMCTL", cmd),
    }

    Ok(SyscallReturn::Return(0))
}

fn check_and_ctl<T, F>(semid: i32, permission: PermissionMode, ctl_func: F) -> Result<T>
where
    F: FnOnce(&SemaphoreSet) -> Result<T>,
{
    check_sem(semid, None, permission)?;
    let sem_sets = sem_sets();
    let sem_set = sem_sets.get(&semid).ok_or(Error::new(Errno::EINVAL))?;
    ctl_func.call_once((sem_set,))
}
