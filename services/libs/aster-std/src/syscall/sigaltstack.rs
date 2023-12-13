use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::PosixThreadExt;
use crate::process::signal::SigStack;
use crate::process::signal::SigStackFlags;
use crate::util::read_val_from_user;
use crate::util::write_val_to_user;

use super::{SyscallReturn, SYS_SIGALTSTACK};

pub fn sys_sigaltstack(sig_stack_addr: Vaddr, old_sig_stack_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SIGALTSTACK);

    debug!(
        "sig_stack_addr = 0x{:x}, old_sig_stack_addr: 0x{:x}",
        sig_stack_addr, old_sig_stack_addr
    );

    let old_stack = {
        let current_thread = current_thread!();
        let posix_thread = current_thread.as_posix_thread().unwrap();
        let sig_stack = posix_thread.sig_stack().lock();
        sig_stack.clone()
    };

    get_old_stack(old_sig_stack_addr, old_stack.as_ref())?;
    set_new_stack(sig_stack_addr, old_stack.as_ref())?;

    Ok(SyscallReturn::Return(0))
}

fn get_old_stack(old_sig_stack_addr: Vaddr, old_stack: Option<&SigStack>) -> Result<()> {
    if old_sig_stack_addr == 0 {
        return Ok(());
    }

    let Some(old_stack) = old_stack else {
        return Ok(());
    };

    debug!("old stack = {:?}", old_stack);

    let stack = stack_t::from(old_stack.clone());
    write_val_to_user(old_sig_stack_addr, &stack)
}

fn set_new_stack(sig_stack_addr: Vaddr, old_stack: Option<&SigStack>) -> Result<()> {
    if sig_stack_addr == 0 {
        return Ok(());
    }

    if let Some(old_stack) = old_stack
        && old_stack.is_active()
    {
        return_errno_with_message!(Errno::EPERM, "the old stack is active now");
    }

    let new_stack = {
        let stack = read_val_from_user::<stack_t>(sig_stack_addr)?;
        SigStack::try_from(stack)?
    };

    debug!("new_stack = {:?}", new_stack);

    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    *posix_thread.sig_stack().lock() = Some(new_stack);

    Ok(())
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct stack_t {
    // Base address of stack
    sp: Vaddr,
    flags: i32,
    // Number of bytes in stack
    size: usize,
}

impl TryFrom<stack_t> for SigStack {
    type Error = Error;

    fn try_from(stack: stack_t) -> Result<Self> {
        if stack.flags < 0 {
            return_errno_with_message!(Errno::EINVAL, "negative flags");
        }

        let flags = SigStackFlags::from_bits(stack.flags as u32)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;

        if stack.size < MINSTKSZ {
            return_errno_with_message!(Errno::ENOMEM, "stack size is less than MINSTKSZ");
        }

        Ok(Self::new(stack.sp, flags, stack.size))
    }
}

impl From<SigStack> for stack_t {
    fn from(stack: SigStack) -> Self {
        let flags = stack.flags().bits() as i32 | stack.status() as i32;
        Self {
            sp: stack.base(),
            flags,
            size: stack.size(),
        }
    }
}

const SIGSTKSZ: usize = 8192;
const MINSTKSZ: usize = 2048;
