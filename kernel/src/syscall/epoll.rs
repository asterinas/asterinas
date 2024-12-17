// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags},
        file_table::{FdFlags, FileDesc},
        utils::CreationFlags,
    },
    prelude::*,
    process::signal::sig_mask::SigMask,
};

// See: https://elixir.bootlin.com/linux/v6.11.5/source/fs/eventpoll.c#L2437
const EP_MAX_EVENTS: usize = i32::MAX as usize / core::mem::size_of::<c_epoll_event>();

pub fn sys_epoll_create(size: i32, ctx: &Context) -> Result<SyscallReturn> {
    if size <= 0 {
        return_errno_with_message!(Errno::EINVAL, "size is not positive");
    }
    sys_epoll_create1(0, ctx)
}

pub fn sys_epoll_create1(flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("flags = 0x{:x}", flags);

    let fd_flags = {
        let flags = CreationFlags::from_bits(flags)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
        if flags == CreationFlags::empty() {
            FdFlags::empty()
        } else if flags == CreationFlags::O_CLOEXEC {
            FdFlags::CLOEXEC
        } else {
            // Only O_CLOEXEC is valid
            return_errno_with_message!(Errno::EINVAL, "invalid flags");
        }
    };

    let epoll_file: Arc<EpollFile> = EpollFile::new();
    let mut file_table = ctx.posix_thread.file_table().lock();
    let fd = file_table.insert(epoll_file, fd_flags);
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_epoll_ctl(
    epfd: FileDesc,
    op: i32,
    fd: FileDesc,
    event_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "epfd = {}, op = {}, fd = {}, event_addr = 0x{:x}",
        epfd, op, fd, event_addr
    );

    const EPOLL_CTL_ADD: i32 = 1;
    const EPOLL_CTL_DEL: i32 = 2;
    const EPOLL_CTL_MOD: i32 = 3;

    let cmd = match op {
        EPOLL_CTL_ADD => {
            let c_epoll_event = ctx.user_space().read_val::<c_epoll_event>(event_addr)?;
            let event = EpollEvent::from(&c_epoll_event);
            let flags = EpollFlags::from_bits_truncate(c_epoll_event.events);
            EpollCtl::Add(fd, event, flags)
        }
        EPOLL_CTL_DEL => EpollCtl::Del(fd),
        EPOLL_CTL_MOD => {
            let c_epoll_event = ctx.user_space().read_val::<c_epoll_event>(event_addr)?;
            let event = EpollEvent::from(&c_epoll_event);
            let flags = EpollFlags::from_bits_truncate(c_epoll_event.events);
            EpollCtl::Mod(fd, event, flags)
        }
        _ => return_errno_with_message!(Errno::EINVAL, "invalid op"),
    };

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(epfd)?.clone()
    };
    let epoll_file = file
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;
    epoll_file.control(&cmd)?;

    Ok(SyscallReturn::Return(0 as _))
}

fn do_epoll_wait(
    epfd: FileDesc,
    max_events: i32,
    timeout: i32,
    ctx: &Context,
) -> Result<Vec<EpollEvent>> {
    let max_events = {
        if max_events <= 0 || max_events as usize > EP_MAX_EVENTS {
            return_errno_with_message!(Errno::EINVAL, "max_events is not valid");
        }
        max_events as usize
    };
    let timeout = if timeout >= 0 {
        Some(Duration::from_millis(timeout as _))
    } else {
        None
    };

    let epoll_file_arc = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(epfd)?.clone()
    };
    let epoll_file = epoll_file_arc
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;
    let result = epoll_file.wait(max_events, timeout.as_ref());

    // As mentioned in the manual, the return value should be zero if no file descriptor becomes ready
    // during the requested `timeout` milliseconds. So we ignore `Err(ETIME)` and return an empty vector.
    //
    // Manual: <https://www.man7.org/linux/man-pages/man2/epoll_wait.2.html>
    if result
        .as_ref()
        .is_err_and(|err| err.error() == Errno::ETIME)
    {
        return Ok(Vec::new());
    }
    result
}

pub fn sys_epoll_wait(
    epfd: FileDesc,
    events_addr: Vaddr,
    max_events: i32,
    timeout: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "epfd = {}, events_addr = 0x{:x}, max_events = {}, timeout = {:?}",
        epfd, events_addr, max_events, timeout
    );

    let epoll_events = do_epoll_wait(epfd, max_events, timeout, ctx)?;

    // Write back
    let mut write_addr = events_addr;
    let user_space = ctx.user_space();
    for epoll_event in epoll_events.iter() {
        let c_epoll_event = c_epoll_event::from(epoll_event);
        user_space.write_val(write_addr, &c_epoll_event)?;
        write_addr += core::mem::size_of::<c_epoll_event>();
    }

    Ok(SyscallReturn::Return(epoll_events.len() as _))
}

fn set_signal_mask(set_ptr: Vaddr, ctx: &Context) -> Result<SigMask> {
    let new_mask: Option<SigMask> = if set_ptr != 0 {
        Some(ctx.user_space().read_val::<u64>(set_ptr)?.into())
    } else {
        None
    };

    let old_sig_mask_value = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);

    if let Some(new_mask) = new_mask {
        ctx.posix_thread
            .sig_mask()
            .store(new_mask, Ordering::Relaxed);
    }

    Ok(old_sig_mask_value)
}

fn restore_signal_mask(sig_mask_val: SigMask, ctx: &Context) {
    ctx.posix_thread
        .sig_mask()
        .store(sig_mask_val, Ordering::Relaxed);
}

pub fn sys_epoll_pwait(
    epfd: FileDesc,
    events_addr: Vaddr,
    max_events: i32,
    timeout: i32,
    sigmask: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "epfd = {}, events_addr = 0x{:x}, max_events = {}, timeout = {:?}, sigmask = 0x{:x}, sigset_size = {}",
        epfd, events_addr, max_events, timeout, sigmask, sigset_size
    );

    if sigmask != 0 && sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8");
    }

    let old_sig_mask_value = set_signal_mask(sigmask, ctx)?;

    let ready_events = match do_epoll_wait(epfd, max_events, timeout, ctx) {
        Ok(events) => {
            restore_signal_mask(old_sig_mask_value, ctx);
            events
        }
        Err(e) => {
            // Restore the signal mask even if an error occurs
            restore_signal_mask(old_sig_mask_value, ctx);
            return Err(e);
        }
    };

    // Write back
    let mut write_addr = events_addr;
    let user_space = ctx.user_space();
    for event in ready_events.iter() {
        let c_event = c_epoll_event::from(event);
        user_space.write_val(write_addr, &c_event)?;
        write_addr += core::mem::size_of::<c_epoll_event>();
    }

    Ok(SyscallReturn::Return(ready_events.len() as _))
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C, packed)]
struct c_epoll_event {
    events: u32,
    data: u64,
}

impl From<&EpollEvent> for c_epoll_event {
    fn from(ep_event: &EpollEvent) -> Self {
        Self {
            events: ep_event.events.bits(),
            data: ep_event.user_data,
        }
    }
}

impl From<&c_epoll_event> for EpollEvent {
    fn from(c_event: &c_epoll_event) -> Self {
        Self::new(IoEvents::from_bits_truncate(c_event.events), c_event.data)
    }
}
