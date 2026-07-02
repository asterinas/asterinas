// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        posix_thread::ContextPthreadAdminApi,
        signal::sig_mask::{SigMask, SigMaskFullSize},
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
    let checked_size = SigMask::check_full_size(sigset_size)?;
    do_rt_sigprocmask(mask_op, set_ptr, oldset_ptr, checked_size, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigprocmask(
    mask_op: MaskOp,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    checked_size: SigMaskFullSize,
    ctx: &Context,
) -> Result<()> {
    let old_sig_mask_value = ctx.posix_thread.sig_mask();
    debug!("old sig mask value: 0x{:x}", old_sig_mask_value);
    if oldset_ptr != 0 {
        checked_size.write_val(&ctx.user_space(), oldset_ptr, &old_sig_mask_value)?;
    }

    if set_ptr != 0 {
        let read_mask = checked_size.read_val(&ctx.user_space(), set_ptr)?;
        match mask_op {
            MaskOp::Block => {
                ctx.set_sig_mask(old_sig_mask_value + read_mask);
            }
            MaskOp::Unblock => {
                ctx.set_sig_mask(old_sig_mask_value - read_mask);
            }
            MaskOp::SetMask => {
                ctx.set_sig_mask(read_mask);
            }
        }
    }
    debug!("new set = {:x?}", ctx.posix_thread.sig_mask());

    Ok(())
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum MaskOp {
    Block = 0,
    Unblock = 1,
    SetMask = 2,
}
