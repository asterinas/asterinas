// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        credentials::capabilities::CapSet, posix_thread::MAX_THREAD_NAME_LEN,
        signal::sig_num::SigNum,
    },
};

pub fn sys_prctl(
    option: i32,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let prctl_cmd = PrctlCmd::from_args(option, arg2, arg3, arg4, arg5)?;
    debug!("prctl cmd = {:x?}", prctl_cmd);
    match prctl_cmd {
        PrctlCmd::PR_SET_PDEATHSIG(signum) => {
            ctx.process.set_parent_death_signal(signum);
        }
        PrctlCmd::PR_GET_PDEATHSIG(write_to_addr) => {
            let write_val = {
                match ctx.process.parent_death_signal() {
                    None => 0i32,
                    Some(signum) => signum.as_u8() as i32,
                }
            };

            ctx.user_space().write_val(write_to_addr, &write_val)?;
        }
        PrctlCmd::PR_GET_DUMPABLE => {
            // TODO: when coredump is supported, return the actual value
            return Ok(SyscallReturn::Return(Dumpable::Disable as _));
        }
        PrctlCmd::PR_SET_DUMPABLE(dumpable) => {
            if dumpable != Dumpable::Disable && dumpable != Dumpable::User {
                return_errno!(Errno::EINVAL)
            }

            // TODO: implement coredump
        }
        PrctlCmd::PR_GET_KEEPCAPS => {
            let keep_cap = {
                let credentials = ctx.posix_thread.credentials();
                if credentials.keep_capabilities() {
                    1
                } else {
                    0
                }
            };

            return Ok(SyscallReturn::Return(keep_cap as _));
        }
        PrctlCmd::PR_SET_KEEPCAPS(keep_cap) => {
            if keep_cap > 1 {
                return_errno!(Errno::EINVAL)
            }
            let credentials = ctx.posix_thread.credentials_mut();
            credentials.set_keep_capabilities(keep_cap != 0);
        }
        PrctlCmd::PR_GET_NAME(write_to_addr) => {
            let thread_name = ctx.posix_thread.thread_name().lock();
            if let Some(thread_name) = &*thread_name {
                if let Some(thread_name) = thread_name.name()? {
                    ctx.user_space().write_bytes(
                        write_to_addr,
                        &mut VmReader::from(thread_name.to_bytes_with_nul()),
                    )?;
                }
            }
        }
        PrctlCmd::PR_SET_NAME(read_addr) => {
            let mut thread_name = ctx.posix_thread.thread_name().lock();
            if let Some(thread_name) = &mut *thread_name {
                let new_thread_name = ctx
                    .user_space()
                    .read_cstring(read_addr, MAX_THREAD_NAME_LEN)?;
                thread_name.set_name(&new_thread_name)?;
            }
        }
        PrctlCmd::PR_CAPBSET_READ(cap_set) => {
            if !cap_valid(cap_set as u64) {
                return_errno!(Errno::EINVAL)
            }
            let cred_bset = ctx.posix_thread.credentials().bounding_capset().as_u64();
            return Ok(SyscallReturn::Return((cred_bset & (1 << cap_set)) as _));
        }
        PrctlCmd::PR_GET_SECUREBITS => {
            let securebits: u32 = ctx.posix_thread.credentials().securebits().into();
            return Ok(SyscallReturn::Return(securebits as _));
        }
        PrctlCmd::PR_SET_CHILD_SUBREAPER(is_set) => {
            let process = ctx.process;
            if is_set {
                process.set_child_subreaper();
            } else {
                process.unset_child_subreaper();
            }
        }
        PrctlCmd::PR_GET_CHILD_SUBREAPER(write_addr) => {
            let process = ctx.process;
            ctx.user_space()
                .write_val(write_addr, &(process.is_child_subreaper() as u32))?;
        }
        PrctlCmd::PR_GET_NO_NEW_PRIVS => {
            // TODO: implement no_new_privs for process.
            // This mechanism should add atomic flags for process.
            // By default, the value obtained is 0.
            return Ok(SyscallReturn::Return(0));
        }
        PrctlCmd::PR_CAP_AMBIENT(cmd) => {
            if cmd == PR_CAP_AMBIENT_CLEAR_ALL {
                // TODO: implement clear all ambient capabilities.
            }
            if !cap_valid(arg3) || arg4 != 0 || arg5 != 0 {
                return_errno!(Errno::EINVAL)
            }
            let cap_set = ctx.posix_thread.credentials().ambient_capset().as_u64();
            if cmd == PR_CAP_AMBIENT_IS_SET {
                return Ok(SyscallReturn::Return((cap_set & (1 << arg3)) as _));
            } else if cmd != PR_CAP_AMBIENT_RAISE && cmd != PR_CAP_AMBIENT_LOWER {
                return_errno!(Errno::EINVAL)
            }
            // TODO: add support for PR_CAP_AMBIENT_RAISE and PR_CAP_AMBIENT_LOWER.
        }
        _ => todo!(),
    }
    Ok(SyscallReturn::Return(0))
}

fn cap_valid(cap: u64) -> bool {
    cap <= CapSet::most_significant_bit() as u64
}

const PR_SET_PDEATHSIG: i32 = 1; // Second arg is a signal.
const PR_GET_PDEATHSIG: i32 = 2; // Second arg is a ptr to return the signal.
const PR_GET_DUMPABLE: i32 = 3; // Get process's dumpable state.
const PR_SET_DUMPABLE: i32 = 4; // Set process's dumpable state.
const PR_GET_KEEPCAPS: i32 = 7; // Get whether or not to drop capabilities on setuid() away from uid 0.
const PR_SET_KEEPCAPS: i32 = 8; // Set whether or not to drop capabilities on setuid() away from uid 0.
const PR_SET_NAME: i32 = 15; // Set process name.
const PR_GET_NAME: i32 = 16; // Get process name.
const PR_CAPBSET_READ: i32 = 23; // Get the capability bounding set.
const PR_GET_SECUREBITS: i32 = 24; // Get securebits.
const PR_SET_TIMERSLACK: i32 = 29; // Set the timerslack as used by poll/select/nanosleep.
const PR_GET_TIMERSLACK: i32 = 30; // Get the timerslack as used by poll/select/nanosleep.
const PR_SET_CHILD_SUBREAPER: i32 = 36; // Set process's child subreaper state.
const PR_GET_CHILD_SUBREAPER: i32 = 37; // Get process's child subreaper state.
const PR_GET_NO_NEW_PRIVS: i32 = 38; // Get process's no new privileges state.
const PR_CAP_AMBIENT: i32 = 47; // Control the ambient capability set.

#[expect(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub enum PrctlCmd {
    PR_SET_PDEATHSIG(SigNum),
    PR_GET_PDEATHSIG(Vaddr),
    PR_SET_NAME(Vaddr),
    PR_GET_NAME(Vaddr),
    PR_GET_KEEPCAPS,
    PR_SET_KEEPCAPS(u32),
    PR_CAPBSET_READ(u32),
    PR_GET_SECUREBITS,
    #[expect(dead_code)]
    PR_SET_TIMERSLACK(u64),
    #[expect(dead_code)]
    PR_GET_TIMERSLACK,
    PR_SET_DUMPABLE(Dumpable),
    PR_GET_DUMPABLE,
    PR_SET_CHILD_SUBREAPER(bool),
    PR_GET_CHILD_SUBREAPER(Vaddr),
    PR_GET_NO_NEW_PRIVS,
    PR_CAP_AMBIENT(u32),
}

const PR_CAP_AMBIENT_IS_SET: u32 = 1; // Check if a capability is set in the ambient set.
const PR_CAP_AMBIENT_RAISE: u32 = 2; // Raise a capability in the ambient set.
const PR_CAP_AMBIENT_LOWER: u32 = 3; // Lower a capability in the ambient set.
const PR_CAP_AMBIENT_CLEAR_ALL: u32 = 4; // Clear all capabilities in the ambient set.

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
pub enum Dumpable {
    Disable = 0, /* No setuid dumping */
    User = 1,    /* Dump as user of process */
    Root = 2,    /* Dump as root */
}

impl PrctlCmd {
    fn from_args(option: i32, arg2: u64, _arg3: u64, _arg4: u64, _arg5: u64) -> Result<PrctlCmd> {
        match option {
            PR_SET_PDEATHSIG => {
                let signum = SigNum::try_from(arg2 as u8)?;
                Ok(PrctlCmd::PR_SET_PDEATHSIG(signum))
            }
            PR_GET_PDEATHSIG => Ok(PrctlCmd::PR_GET_PDEATHSIG(arg2 as _)),
            PR_GET_DUMPABLE => Ok(PrctlCmd::PR_GET_DUMPABLE),
            PR_SET_DUMPABLE => Ok(PrctlCmd::PR_SET_DUMPABLE(Dumpable::try_from(arg2)?)),
            PR_SET_NAME => Ok(PrctlCmd::PR_SET_NAME(arg2 as _)),
            PR_GET_NAME => Ok(PrctlCmd::PR_GET_NAME(arg2 as _)),
            PR_CAPBSET_READ => Ok(PrctlCmd::PR_CAPBSET_READ(arg2 as _)),
            PR_GET_SECUREBITS => Ok(PrctlCmd::PR_GET_SECUREBITS),
            PR_GET_TIMERSLACK => todo!(),
            PR_SET_TIMERSLACK => todo!(),
            PR_GET_KEEPCAPS => Ok(PrctlCmd::PR_GET_KEEPCAPS),
            PR_SET_KEEPCAPS => Ok(PrctlCmd::PR_SET_KEEPCAPS(arg2 as _)),
            PR_SET_CHILD_SUBREAPER => Ok(PrctlCmd::PR_SET_CHILD_SUBREAPER(arg2 > 0)),
            PR_GET_CHILD_SUBREAPER => Ok(PrctlCmd::PR_GET_CHILD_SUBREAPER(arg2 as _)),
            PR_GET_NO_NEW_PRIVS => Ok(PrctlCmd::PR_GET_NO_NEW_PRIVS),
            PR_CAP_AMBIENT => Ok(PrctlCmd::PR_CAP_AMBIENT(arg2 as _)),
            _ => {
                debug!("prctl cmd number: {}", option);
                return_errno_with_message!(Errno::EINVAL, "unsupported prctl command");
            }
        }
    }
}
