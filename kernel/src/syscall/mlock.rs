// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use align_ext::AlignExt;
use bitflags::bitflags;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{FutureMemoryLock, ResourceType, credentials::capabilities::CapSet},
    security::lsm::hooks as lsm_hooks,
    vm::vmar::VMAR_CAP_ADDR,
};

bitflags! {
    struct MLockAllFlags: u32 {
        const MCL_CURRENT = 1;
        const MCL_FUTURE = 2;
        const MCL_ONFAULT = 4;
    }
}

pub fn sys_mlock(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let range = check_range(addr, len)?;
    if range.is_empty() {
        return Ok(SyscallReturn::Return(0));
    }

    let limit = memlock_limit(ctx);
    ctx.user_space().vmar().lock_range(range, limit)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_munlock(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let range = check_range(addr, len)?;
    if range.is_empty() {
        return Ok(SyscallReturn::Return(0));
    }

    ctx.user_space().vmar().unlock_range(range)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_mlockall(flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = MLockAllFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown mlockall flags"))?;

    if flags.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "invalid mlockall flags");
    }

    if flags.contains(MLockAllFlags::MCL_ONFAULT) {
        return_errno_with_message!(Errno::EINVAL, "MCL_ONFAULT is not supported");
    }

    let limit = memlock_limit(ctx);
    let future_memory_lock = if flags.contains(MLockAllFlags::MCL_FUTURE) {
        FutureMemoryLock::Enabled
    } else {
        FutureMemoryLock::Disabled
    };
    ctx.user_space().vmar().lock_all(
        flags.contains(MLockAllFlags::MCL_CURRENT),
        future_memory_lock,
        limit,
    )?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_munlockall(ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    vmar.unlock_all();

    Ok(SyscallReturn::Return(0))
}

fn check_range(addr: Vaddr, len: usize) -> Result<Range<Vaddr>> {
    if len == 0 {
        return Ok(addr..addr);
    }

    let start = addr.align_down(PAGE_SIZE);
    let end = addr
        .checked_add(len)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the memory range overflows"))?;
    let end = if end.is_multiple_of(PAGE_SIZE) {
        end
    } else {
        end.checked_add(PAGE_SIZE - 1)
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "the memory range overflows"))?
            .align_down(PAGE_SIZE)
    };

    if end > VMAR_CAP_ADDR {
        return_errno_with_message!(Errno::ENOMEM, "the memory range is not in userspace");
    }

    Ok(start..end)
}

pub(super) fn memlock_limit(ctx: &Context) -> Option<usize> {
    if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        CapSet::IPC_LOCK,
    ))
    .is_ok()
    {
        return None;
    }

    let limit = ctx
        .posix_thread
        .process()
        .resource_limits()
        .get_rlimit(ResourceType::RLIMIT_MEMLOCK)
        .get_cur();

    usize::try_from(limit).ok()
}
