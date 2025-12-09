// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use ostd::{mm::VmIo, sync::Waiter};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{
        HandlePendingSignal,
        constants::{SIGKILL, SIGSTOP},
        sig_mask::{SigMask, SigSet},
        signals::Signal,
        with_sigmask_changed,
    },
    time::{timespec_t, wait::ManagedTimeout},
};

pub fn sys_rt_sigtimedwait(
    set_ptr: Vaddr,
    info_ptr: Vaddr,
    timeout_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "set_ptr = {:#?}, info_ptr = {:#?}, timeout_ptr = {:#?}, sigset_size = {}",
        set_ptr, info_ptr, timeout_ptr, sigset_size
    );

    // Validate sigset size
    if sigset_size != size_of::<SigMask>() {
        return_errno_with_message!(Errno::EINVAL, "invalid sigset size");
    }

    // Read the signal set
    let mask = {
        let mut set: SigSet = ctx.user_space().read_val(set_ptr)?;

        // Remove SIGKILL and SIGSTOP as they cannot be waited for
        set -= SIGKILL;
        set -= SIGSTOP;

        !set
    };

    // Read timeout if provided
    let timeout = if timeout_ptr != 0 {
        let timespec: timespec_t = ctx.user_space().read_val(timeout_ptr)?;
        Some(Duration::try_from(timespec)?)
    } else {
        None
    };

    debug!(
        "pid = {}, sig_mask = {:?}, timeout = {:?}",
        ctx.process.pid(),
        mask,
        timeout
    );

    let block_list = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);
    // Fast path: If a signal is already pending, dequeue and return it immediately.
    if let Some(signal) = dequeue_signal_with_checking_ignore(ctx, mask, block_list) {
        if info_ptr != 0 {
            let siginfo = signal.to_info();
            ctx.user_space().write_val(info_ptr, &siginfo)?;
        }

        return Ok(SyscallReturn::Return(signal.num().as_u8() as _));
    }

    with_sigmask_changed(
        ctx,
        |sig_mask| sig_mask & mask,
        || {
            // Wait for a signal to arrive or timeout.
            let waiter = Waiter::new_pair().0;
            let signal = waiter
                .pause_until_or_timeout(
                    || dequeue_signal_with_checking_ignore(ctx, mask, block_list),
                    timeout.map(ManagedTimeout::new),
                )
                .map_err(|e| {
                    if e.error() == Errno::ETIME {
                        Error::new(Errno::EAGAIN)
                    } else {
                        e
                    }
                })?;

            if info_ptr != 0 {
                let siginfo = signal.to_info();
                ctx.user_space().write_val(info_ptr, &siginfo)?;
            }

            Ok(SyscallReturn::Return(signal.num().as_u8() as _))
        },
    )
}

/// Dequeue a signal from the thread's pending signal queue.
///
/// If the signal is ignored and not blocked, it will be dropped and
/// the next signal will be checked.
fn dequeue_signal_with_checking_ignore(
    ctx: &Context,
    mask: SigMask,
    block_list: SigMask,
) -> Option<Box<dyn Signal>> {
    let sig_dispositions = ctx.process.sig_dispositions().lock();
    let sig_disposition = sig_dispositions.lock();
    while let Some(signal) = ctx.posix_thread.dequeue_signal(&mask) {
        // If the signal is ignored and not blocked, the signal will be directly dropped.
        if !block_list.contains(signal.num()) && sig_disposition.will_ignore(signal.as_ref()) {
            continue;
        }

        return Some(signal);
    }

    None
}
