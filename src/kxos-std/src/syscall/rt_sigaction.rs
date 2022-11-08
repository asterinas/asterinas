use crate::{
    memory::{read_val_from_user, write_val_to_user},
    prelude::*,
    process::signal::{c_types::sigaction_t, sig_action::SigAction, sig_num::SigNum},
    syscall::SYS_RT_SIGACTION,
};

use super::SyscallReturn;

pub fn sys_rt_sigaction(
    sig_num: u8,
    sig_action_ptr: Vaddr,
    old_sig_action_ptr: Vaddr,
    sigset_size: u64,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_RT_SIGACTION]", SYS_RT_SIGACTION);
    let sig_num = SigNum::try_from(sig_num)?;
    debug!("sig_num = {}", sig_num.sig_name());
    debug!("sig_action_ptr = 0x{:x}", sig_action_ptr);
    debug!("old_sig_action_ptr = 0x{:x}", old_sig_action_ptr);
    debug!("sigset_size = {}", sigset_size);
    let sig_action_c = read_val_from_user::<sigaction_t>(sig_action_ptr)?;
    debug!("sig_action_c = {:?}", sig_action_c);
    let sig_action = SigAction::try_from(sig_action_c).unwrap();
    debug!("sig_action = {:x?}", sig_action);

    let current = current!();
    let mut sig_dispositions = current.sig_dispositions().lock();
    let old_action = sig_dispositions.get(sig_num);
    debug!("old_action = {:x?}", old_action);
    let old_action_c = old_action.to_c();
    debug!("old_action_c = {:x?}", old_action_c);
    sig_dispositions.set(sig_num, sig_action);
    if old_sig_action_ptr != 0 {
        write_val_to_user(old_sig_action_ptr, &old_action_c)?;
    }
    Ok(SyscallReturn::Return(0))
}
