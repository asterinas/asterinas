// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{
        sem_set::{sem_sets, sem_sets_mut},
        IpcControlCmd,
    },
    prelude::*,
};

pub fn sys_semctl(id: i32, semnum: i32, cmd: i32, arg: Vaddr) -> Result<SyscallReturn> {
    if id <= 0 || semnum < 0 {
        return_errno!(Errno::EINVAL)
    }

    let cmd = IpcControlCmd::try_from(cmd)?;
    debug!(
        "[sys_semctl] id= {}, semnum = {}, cmd = {:?}, arg = {:x}",
        id, semnum, cmd, arg
    );

    match cmd {
        IpcControlCmd::IPC_RMID => {
            let mut sem_sets_mut = sem_sets_mut();
            sem_sets_mut.remove(&id).ok_or(Error::new(Errno::EINVAL))?;
        }
        IpcControlCmd::SEM_SETVAL => {
            // In setval, arg is parse as i32
            let val = arg as i32;
            if val < 0 {
                return_errno!(Errno::ERANGE);
            }

            let sem_sets = sem_sets();
            let sem_set = sem_sets.get(&id).ok_or(Error::new(Errno::EINVAL))?;
            let sem = sem_set
                .get(semnum as usize)
                .ok_or(Error::new(Errno::EINVAL))?;

            sem.set_val(val)?;
            sem_set.update_ctime();
        }
        _ => todo!("Support {:?}", cmd),
    }

    Ok(SyscallReturn::Return(0))
}
