// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            c_types::sigaction_t,
            constants::{SIGKILL, SIGSTOP},
            sig_action::SigAction,
            sig_mask::SigSet,
            sig_num::SigNum,
        },
    },
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

    if sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8");
    }

    let mut sig_dispositions = ctx.process.sig_dispositions().lock();

    let old_action = if sig_action_addr != 0 {
        if sig_num == SIGKILL || sig_num == SIGSTOP {
            return_errno_with_message!(
                Errno::EINVAL,
                "cannot set a new signal action for SIGKILL and SIGSTOP"
            );
        }

        let sig_action_c = ctx.user_space().read_val::<sigaction_t>(sig_action_addr)?;
        let sig_action = SigAction::from(sig_action_c);
        trace!("sig action = {:?}", sig_action);
        discard_signals_if_ignored(ctx, sig_num, &sig_action);
        sig_dispositions.set(sig_num, sig_action)?
    } else {
        sig_dispositions.get(sig_num)
    };

    if old_sig_action_addr != 0 {
        let old_action_c = old_action.as_c_type();
        ctx.user_space()
            .write_val(old_sig_action_addr, &old_action_c)?;
    }

    Ok(SyscallReturn::Return(0))
}

/// Discard signals if the new action is to ignore the signal.
///
/// Ref: <https://elixir.bootlin.com/linux/v6.13/source/kernel/signal.c#L4323>
//
// POSIX 3.3.1.3:
// Setting a signal action to SIG_IGN for a signal that is
// pending shall cause the pending signal to be discarded,
// whether or not it is blocked.
//
// Setting a signal action to SIG_DFL for a signal that is
// pending and whose default action is to ignore the signal
// (for example, SIGCHLD), shall cause the pending signal to
// be discarded, whether or not it is blocked
fn discard_signals_if_ignored(ctx: &Context, signum: SigNum, sig_action: &SigAction) {
    if !sig_action.will_ignore(signum) {
        return;
    }

    let mask = SigSet::new_full() - signum;

    for task in ctx.process.tasks().lock().as_slice() {
        let Some(posix_thread) = task.as_posix_thread() else {
            continue;
        };

        posix_thread.dequeue_signal(&mask);
    }
}
