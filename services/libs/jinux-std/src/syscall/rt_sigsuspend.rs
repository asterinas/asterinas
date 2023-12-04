use super::{SyscallReturn, SYS_RT_SIGSUSPEND};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::PosixThreadExt;
use crate::process::signal::sig_mask::SigMask;
use crate::process::signal::Pauser;
use crate::util::read_val_from_user;

pub fn sys_rt_sigsuspend(sigmask_addr: Vaddr, sigmask_size: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RT_SIGSUSPEND);
    debug!(
        "sigmask_addr = 0x{:x}, sigmask_size = {}",
        sigmask_addr, sigmask_size
    );

    debug_assert!(sigmask_size == core::mem::size_of::<SigMask>());
    if sigmask_size != core::mem::size_of::<SigMask>() {
        return_errno_with_message!(Errno::EINVAL, "invalid sigmask size");
    }

    // Set sigmask of current thread
    let sigmask: SigMask = read_val_from_user(sigmask_addr)?;
    let current_thread = current_thread!();
    let poxis_thread = current_thread.as_posix_thread().unwrap();
    *poxis_thread.sig_mask().lock() = sigmask;

    // Pause until receiving any signal
    let pauser = Pauser::new();
    pauser.pause_until(|| None::<()>)?;

    // This syscall should always return `Err(EINTR)`. This path should never be reached.
    debug_assert!(false);
    Ok(SyscallReturn::Return(0))
}
