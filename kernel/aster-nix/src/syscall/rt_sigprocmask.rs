// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::sync::atomic::Ordering;

use ostd::mm::VmIo;

use super::{CallingThreadInfo, SyscallReturn};
use crate::{
    prelude::*,
    process::{
        posix_thread::PosixThreadExt,
        signal::{
            constants::{SIGKILL, SIGSTOP},
            sig_mask::SigSet,
        },
    },
    util::{read_val_from_user, write_val_to_user},
};

pub fn sys_rt_sigprocmask(
    how: u32,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
    info: CallingThreadInfo,
) -> Result<SyscallReturn> {
    let mask_op = MaskOp::try_from(how).unwrap();
    debug!(
        "mask op = {:?}, set_ptr = 0x{:x}, oldset_ptr = 0x{:x}, sigset_size = {}",
        mask_op, set_ptr, oldset_ptr, sigset_size
    );
    if sigset_size != 8 {
        error!("sigset size is not equal to 8");
    }
    do_rt_sigprocmask(mask_op, set_ptr, oldset_ptr, sigset_size, info).unwrap();
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigprocmask(
    mask_op: MaskOp,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
    info: CallingThreadInfo,
) -> Result<()> {
    let sig_mask = &info.pthread_info.sig_mask;
    /// There should be no race here: only the current thread can modify its
    /// own signal mask. Other threads may only read. So the value of the mask
    /// is not possible to change afther the write to user space and before the
    /// modification of the mask.
    let old_sig_mask_value: u64 = sig_mask.load(Ordering::Relaxed).into();
    debug!("old sig mask value: 0x{:x}", old_sig_mask_value);
    if oldset_ptr != 0 {
        write_val_to_user(oldset_ptr, &old_sig_mask_value)?;
    }
    if set_ptr != 0 {
        let new_set = read_val_from_user::<u64>(set_ptr)?;
        match mask_op {
            MaskOp::Block => {
                let mut new_sig_mask = SigSet::from(new_set);
                // According to man pages, "it is not possible to block SIGKILL or SIGSTOP.
                // Attempts to do so are silently ignored."
                new_sig_mask.remove_signal(SIGKILL);
                new_sig_mask.remove_signal(SIGSTOP);
                sig_mask.block(new_sig_mask);
            }
            MaskOp::Unblock => sig_mask.unblock(new_set),
            MaskOp::SetMask => sig_mask.reset(new_set),
        }
    }
    debug!("new set = {:x?}", &sig_mask);

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(u32)]
pub enum MaskOp {
    Block = 0,
    Unblock = 1,
    SetMask = 2,
}
