use crate::memory::read_cstring_from_user;
use crate::memory::write_bytes_to_user;
use crate::prelude::*;
use crate::process::name::MAX_PROCESS_NAME_LEN;

use super::SyscallReturn;
use super::SYS_PRCTL;
pub fn sys_prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_PRCTL]", SYS_PRCTL);
    let prctl_cmd = PrctlCmd::from_args(option, arg2, arg3, arg4, arg5)?;
    debug!("prctl cmd = {:?}", prctl_cmd);
    let current = current!();
    match prctl_cmd {
        PrctlCmd::PR_GET_NAME(write_to_addr) => {
            let process_name = current.process_name().lock();
            if let Some(process_name) = &*process_name {
                if let Some(process_name) = process_name.get_name()? {
                    write_bytes_to_user(write_to_addr, process_name.to_bytes_with_nul())?;
                }
            }
        }
        PrctlCmd::PR_SET_NAME(read_addr) => {
            let mut process_name = current.process_name().lock();
            if let Some(process_name) = &mut *process_name {
                let new_process_name = read_cstring_from_user(read_addr, MAX_PROCESS_NAME_LEN)?;
                process_name.set_name(&new_process_name)?;
            }
        }
        _ => todo!(),
    }
    Ok(SyscallReturn::Return(0))
}

const PR_SET_NAME: i32 = 15;
const PR_GET_NAME: i32 = 16;
const PR_SET_TIMERSLACK: i32 = 29;
const PR_GET_TIMERSLACK: i32 = 30;

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub enum PrctlCmd {
    PR_SET_NAME(Vaddr),
    PR_GET_NAME(Vaddr),
    PR_SET_TIMERSLACK(u64),
    PR_GET_TIMERSLACK,
}

impl PrctlCmd {
    fn from_args(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> Result<PrctlCmd> {
        match option {
            PR_SET_NAME => Ok(PrctlCmd::PR_SET_NAME(arg2 as _)),
            PR_GET_NAME => Ok(PrctlCmd::PR_GET_NAME(arg2 as _)),
            PR_GET_TIMERSLACK => todo!(),
            PR_SET_TIMERSLACK => todo!(),
            _ => {
                debug!("prctl cmd number: {}", option);
                return_errno_with_message!(Errno::EINVAL, "unsupported prctl command");
            }
        }
    }
}
