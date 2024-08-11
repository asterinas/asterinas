// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{c_types::sigaction_t, sig_action::SigAction, sig_num::SigNum},
};

pub fn sys_rt_sigaction(
    sig_num: u8,
    sig_action_addr: Vaddr,
    old_sig_action_addr: Vaddr,
    sigset_size: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let sig_num = SigNum::try_from(sig_num)?;
    debug!(
        "signal = {}, sig_action_addr = 0x{:x}, old_sig_action_addr = 0x{:x}, sigset_size = {}",
        sig_num.sig_name(),
        sig_action_addr,
        old_sig_action_addr,
        sigset_size
    );
    let mut sig_dispositions = ctx.process.sig_dispositions().lock();
    let old_action = sig_dispositions.get(sig_num);
    let old_action_c = old_action.as_c_type();
    if old_sig_action_addr != 0 {
        ctx.get_user_space()
            .write_val(old_sig_action_addr, &old_action_c)?;
    }
    if sig_action_addr != 0 {
        let sig_action_c = ctx
            .get_user_space()
            .read_val::<sigaction_t>(sig_action_addr)?;
        let sig_action = SigAction::try_from(sig_action_c).unwrap();
        trace!("sig action = {:?}", sig_action);
        sig_dispositions.set(sig_num, sig_action);
    }

    Ok(SyscallReturn::Return(0))
}
