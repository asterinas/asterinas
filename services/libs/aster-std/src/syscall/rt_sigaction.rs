use crate::{
    log_syscall_entry,
    prelude::*,
    process::signal::{c_types::sigaction_t, sig_action::SigAction, sig_num::SigNum},
    syscall::SYS_RT_SIGACTION,
    util::{read_val_from_user, write_val_to_user},
};

use super::SyscallReturn;

pub fn sys_rt_sigaction(
    sig_num: u8,
    sig_action_addr: Vaddr,
    old_sig_action_addr: Vaddr,
    sigset_size: u64,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RT_SIGACTION);
    let sig_num = SigNum::try_from(sig_num)?;
    debug!(
        "signal = {}, sig_action_addr = 0x{:x}, old_sig_action_addr = 0x{:x}, sigset_size = {}",
        sig_num.sig_name(),
        sig_action_addr,
        old_sig_action_addr,
        sigset_size
    );
    let current = current!();
    let mut sig_dispositions = current.sig_dispositions().lock();
    let old_action = sig_dispositions.get(sig_num);
    let old_action_c = old_action.as_c_type();
    if old_sig_action_addr != 0 {
        write_val_to_user(old_sig_action_addr, &old_action_c)?;
    }
    if sig_action_addr != 0 {
        let sig_action_c = read_val_from_user::<sigaction_t>(sig_action_addr)?;
        let sig_action = SigAction::try_from(sig_action_c).unwrap();
        trace!("sig action = {:?}", sig_action);
        sig_dispositions.set(sig_num, sig_action);
    }

    Ok(SyscallReturn::Return(0))
}
