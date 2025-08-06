// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::context::UserContext, user::UserContextApi};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{c_types::stack_t, SigStack, SigStackFlags, SigStackStatus},
};

pub fn sys_sigaltstack(
    sig_stack_addr: Vaddr,
    old_sig_stack_addr: Vaddr,
    ctx: &Context,
    user_ctx: &UserContext,
) -> Result<SyscallReturn> {
    debug!(
        "sig_stack_addr = 0x{:x}, old_sig_stack_addr: 0x{:x}",
        sig_stack_addr, old_sig_stack_addr
    );

    let sp = user_ctx.stack_pointer();

    if old_sig_stack_addr != 0 {
        let stack = get_old_stack(ctx, sp)?;
        ctx.user_space()
            .write_val::<stack_t>(old_sig_stack_addr, &stack)?;
    }

    if sig_stack_addr != 0 {
        let stack = ctx.user_space().read_val::<stack_t>(sig_stack_addr)?;
        set_new_stack(stack, ctx, sp)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn get_old_stack(ctx: &Context, sp: usize) -> Result<stack_t> {
    let old_stack = ctx.thread_local.sig_stack().borrow();

    let flags = {
        let attr_flags = SigStackAttrFlags::from_bits_truncate(old_stack.flags().bits());
        let status_flags = SigStackStatusFlags::from(old_stack.active_status(sp));
        attr_flags.bits() | status_flags.bits()
    };

    let stack = stack_t {
        ss_sp: old_stack.base(),
        ss_flags: flags.cast_signed(),
        ss_size: old_stack.size(),
    };

    Ok(stack)
}

pub(super) fn set_new_stack(stack: stack_t, ctx: &Context, sp: usize) -> Result<()> {
    let mut old_stack = ctx.thread_local.sig_stack().borrow_mut();

    if old_stack.contains(sp) {
        return_errno_with_message!(Errno::EPERM, "the old stack is active now");
    }

    let flags = check_new_ss_flags(stack.ss_flags as u32)?;

    let new_stack = if flags.contains(SigStackFlags::SS_DISABLE) {
        SigStack::new(0, flags, 0)
    } else {
        if stack.ss_size < MINSTKSZ {
            return_errno_with_message!(Errno::ENOMEM, "stack size is less than MINSTKSZ");
        }

        if stack.ss_sp.checked_add(stack.ss_size).is_none() {
            return_errno_with_message!(Errno::EINVAL, "overflow for given stack addr and size");
        }

        SigStack::new(stack.ss_sp, flags, stack.ss_size)
    };

    debug!("new stack = {:x?}", new_stack);

    *old_stack = new_stack;

    Ok(())
}

fn check_new_ss_flags(ss_flags: u32) -> Result<SigStackFlags> {
    let ss_flags = SigStackFlags::from_bits(ss_flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown signal stack flags"))?;

    let status_flags = SigStackStatusFlags::from_bits_truncate(ss_flags.bits());

    // Linux permits SS_ONSTACK to be set on a new stack, so we follow Linux's behavior here.
    // However, this may be considered a BUG.
    // Reference: <https://man7.org/linux/man-pages/man2/sigaltstack.2.html#BUGS>.
    if status_flags != SigStackStatusFlags::SS_DISABLE
        && status_flags != SigStackStatusFlags::SS_ONSTACK
        && status_flags != SigStackStatusFlags::empty()
    {
        return_errno_with_message!(Errno::EINVAL, "invalid signal stack flags")
    }

    Ok(ss_flags)
}

#[expect(unused)]
const SIGSTKSZ: usize = 8192;
const MINSTKSZ: usize = 2048;

// The signal stack flags are categorized into two classes: attribute flags and status flags.
//
// They behave differently for access and modification:
// - When retrieving flags, only user-set attribute flags are visible. Status flags
//   are managed internally to reflect the stack's operational state.
// - When setting flags, multiple attributes can be combined, but at most one
//   status flag may be set.

bitflags! {
    struct SigStackAttrFlags: u32 {
        const SS_AUTODISARM = SigStackFlags::SS_AUTODISARM.bits();
    }
}

bitflags! {
    struct SigStackStatusFlags: u32 {
        const SS_ONSTACK = SigStackFlags::SS_ONSTACK.bits();
        const SS_DISABLE = SigStackFlags::SS_DISABLE.bits();
    }
}

impl From<SigStackStatus> for SigStackStatusFlags {
    fn from(value: SigStackStatus) -> Self {
        match value {
            SigStackStatus::Inactive => SigStackStatusFlags::empty(),
            SigStackStatus::Active => SigStackStatusFlags::SS_ONSTACK,
            SigStackStatus::Disable => SigStackStatusFlags::SS_DISABLE,
        }
    }
}
