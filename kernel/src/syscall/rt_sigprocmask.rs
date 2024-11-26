// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{
        constants::{SIGKILL, SIGSTOP},
        sig_mask::SigMask,
    },
};

pub fn sys_rt_sigprocmask(
    how: u32,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mask_op = MaskOp::try_from(how)?;
    debug!(
        "mask op = {:?}, set_ptr = 0x{:x}, oldset_ptr = 0x{:x}, sigset_size = {}",
        mask_op, set_ptr, oldset_ptr, sigset_size
    );
    if sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8");
    }
    do_rt_sigprocmask(mask_op, set_ptr, oldset_ptr, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigprocmask(
    mask_op: MaskOp,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    ctx: &Context,
) -> Result<()> {
    let old_sig_mask_value = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);
    debug!("old sig mask value: 0x{:x}", old_sig_mask_value);
    if oldset_ptr != 0 {
        ctx.user_space()
            .write_val(oldset_ptr, &old_sig_mask_value)?;
    }

    let sig_mask_ref = ctx.posix_thread.sig_mask();
    if set_ptr != 0 {
        let mut read_mask = ctx.user_space().read_val::<SigMask>(set_ptr)?;
        match mask_op {
            MaskOp::Block => {
                // According to man pages, "it is not possible to block SIGKILL or SIGSTOP.
                // Attempts to do so are silently ignored."
                read_mask -= SIGKILL;
                read_mask -= SIGSTOP;
                sig_mask_ref.store(old_sig_mask_value + read_mask, Ordering::Relaxed);
            }
            MaskOp::Unblock => {
                sig_mask_ref.store(old_sig_mask_value - read_mask, Ordering::Relaxed)
            }
            MaskOp::SetMask => sig_mask_ref.store(read_mask, Ordering::Relaxed),
        }
    }
    debug!("new set = {:x?}", sig_mask_ref.load(Ordering::Relaxed));

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(u32)]
pub enum MaskOp {
    Block = 0,
    Unblock = 1,
    SetMask = 2,
}
