// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_RT_SIGPROCMASK};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::PosixThreadExt;
use crate::process::signal::constants::{SIGKILL, SIGSTOP};
use crate::process::signal::sig_mask::SigMask;
use aster_frame::vm::VmIo;

pub fn sys_rt_sigprocmask(
    how: u32,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RT_SIGPROCMASK);
    let mask_op = MaskOp::try_from(how).unwrap();
    debug!(
        "mask op = {:?}, set_ptr = 0x{:x}, oldset_ptr = 0x{:x}, sigset_size = {}",
        mask_op, set_ptr, oldset_ptr, sigset_size
    );
    if sigset_size != 8 {
        error!("sigset size is not equal to 8");
    }
    do_rt_sigprocmask(mask_op, set_ptr, oldset_ptr, sigset_size).unwrap();
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigprocmask(
    mask_op: MaskOp,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
) -> Result<()> {
    let current = current!();
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let root_vmar = current.root_vmar();
    let mut sig_mask = posix_thread.sig_mask().lock();
    let old_sig_mask_value = sig_mask.as_u64();
    debug!("old sig mask value: 0x{:x}", old_sig_mask_value);
    if oldset_ptr != 0 {
        root_vmar.write_val(oldset_ptr, &old_sig_mask_value)?;
    }
    if set_ptr != 0 {
        let new_set = root_vmar.read_val::<u64>(set_ptr)?;
        match mask_op {
            MaskOp::Block => {
                let mut new_sig_mask = SigMask::from(new_set);
                // According to man pages, "it is not possible to block SIGKILL or SIGSTOP.
                // Attempts to do so are silently ignored."
                new_sig_mask.remove_signal(SIGKILL);
                new_sig_mask.remove_signal(SIGSTOP);
                sig_mask.block(new_sig_mask.as_u64());
            }
            MaskOp::Unblock => sig_mask.unblock(new_set),
            MaskOp::SetMask => sig_mask.set(new_set),
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
