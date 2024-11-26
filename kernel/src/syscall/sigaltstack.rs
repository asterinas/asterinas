// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{SigStack, SigStackFlags},
};

pub fn sys_sigaltstack(
    sig_stack_addr: Vaddr,
    old_sig_stack_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "sig_stack_addr = 0x{:x}, old_sig_stack_addr: 0x{:x}",
        sig_stack_addr, old_sig_stack_addr
    );

    let old_stack = {
        let sig_stack = ctx.posix_thread.sig_stack().lock();
        sig_stack.clone()
    };

    get_old_stack(old_sig_stack_addr, old_stack.as_ref(), ctx)?;
    set_new_stack(sig_stack_addr, old_stack.as_ref(), ctx)?;

    Ok(SyscallReturn::Return(0))
}

fn get_old_stack(
    old_sig_stack_addr: Vaddr,
    old_stack: Option<&SigStack>,
    ctx: &Context,
) -> Result<()> {
    if old_sig_stack_addr == 0 {
        return Ok(());
    }

    if let Some(old_stack) = old_stack {
        debug!("old stack = {:?}", old_stack);

        let stack = stack_t::from(old_stack.clone());
        ctx.user_space()
            .write_val::<stack_t>(old_sig_stack_addr, &stack)?;
    } else {
        let stack = stack_t {
            sp: 0,
            flags: SigStackFlags::SS_DISABLE.bits() as i32,
            size: 0,
        };
        ctx.user_space()
            .write_val::<stack_t>(old_sig_stack_addr, &stack)?;
    }

    Ok(())
}

fn set_new_stack(sig_stack_addr: Vaddr, old_stack: Option<&SigStack>, ctx: &Context) -> Result<()> {
    if sig_stack_addr == 0 {
        return Ok(());
    }

    if let Some(old_stack) = old_stack
        && old_stack.is_active()
    {
        return_errno_with_message!(Errno::EPERM, "the old stack is active now");
    }

    let new_stack = {
        let stack = ctx.user_space().read_val::<stack_t>(sig_stack_addr)?;
        SigStack::try_from(stack)?
    };

    debug!("new_stack = {:?}", new_stack);

    *ctx.posix_thread.sig_stack().lock() = Some(new_stack);

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

        let mut flags = SigStackFlags::from_bits(stack.flags as u32)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;

        if flags.contains(SigStackFlags::SS_DISABLE) {
            return Ok(Self::new(0, flags, 0));
        }
        if stack.size < MINSTKSZ {
            return_errno_with_message!(Errno::ENOMEM, "stack size is less than MINSTKSZ");
        }

        if flags.is_empty() {
            flags.insert(SigStackFlags::SS_ONSTACK);
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

#[allow(unused)]
const SIGSTKSZ: usize = 8192;
const MINSTKSZ: usize = 2048;
