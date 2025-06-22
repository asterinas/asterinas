// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags},
        file_table::{get_file_fast, FdFlags, FileDesc},
        utils::CreationFlags,
    },
    prelude::*,
    process::signal::sig_mask::{SigMask, SigSet},
    time::timespec_t,
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
    let file_table = ctx.thread_local.borrow_file_table();
    let fd = file_table.unwrap().write().insert(epoll_file, fd_flags);
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

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, epfd).into_owned();
    // Drop `file_table` as `EpollFile::control` also performs `borrow_file_table_mut()`.
    drop(file_table);

    let epoll_file = file
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;
    epoll_file.control(ctx.thread_local, &cmd)?;

    Ok(SyscallReturn::Return(0 as _))
}

fn do_epoll_pwait2(
    epfd: FileDesc,
    events_addr: Vaddr,
    max_events: i32,
    timeout: Option<Duration>,
    sigmask: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<usize> {
    let max_events = {
        if max_events <= 0 || max_events as usize > EP_MAX_EVENTS {
            return_errno_with_message!(Errno::EINVAL, "max_events is not valid");
        }
        max_events as usize
    };

    let sigset = sigmask != 0;
    if sigset && sigset_size != 8 {
        return_errno_with_message!(Errno::EINVAL, "sigset size is not equal to 8");
    }

    let old_sig_mask_value = if sigset {
        set_signal_mask(sigmask, ctx)?
    } else {
        SigSet::from(0)
    };

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, epfd);
    let epoll_file = file
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;

    let result = epoll_file.wait(max_events, timeout.as_ref());

    if sigset {
        restore_signal_mask(old_sig_mask_value, ctx);
    }

    // As mentioned in the manual, the return value should be zero if no file descriptor becomes ready
    // during the requested `timeout` milliseconds. So we ignore `Err(ETIME)` and return an empty vector.
    //
    // Manual: <https://www.man7.org/linux/man-pages/man2/epoll_wait.2.html>
    let epoll_events = match result {
        Ok(events) => events,
        Err(e) if e.error() == Errno::ETIME => {
            return Ok(0);
        }
        Err(e) => {
            return Err(e);
        }
    };

    // Write back
    let mut write_addr = events_addr;
    let user_space = ctx.user_space();
    for epoll_event in epoll_events.iter() {
        let c_epoll_event = c_epoll_event::from(epoll_event);
        user_space.write_val(write_addr, &c_epoll_event)?;
        write_addr += core::mem::size_of::<c_epoll_event>();
    }

    Ok(epoll_events.len())
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

    let timeout = if timeout >= 0 {
        Some(Duration::from_millis(timeout as _))
    } else {
        None
    };

    let events_len = do_epoll_pwait2(epfd, events_addr, max_events, timeout, 0, 0, ctx)?;

    Ok(SyscallReturn::Return(events_len as _))
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

    let timeout = if timeout >= 0 {
        Some(Duration::from_millis(timeout as _))
    } else {
        None
    };

    let events_len = do_epoll_pwait2(
        epfd,
        events_addr,
        max_events,
        timeout,
        sigmask,
        sigset_size,
        ctx,
    )?;

    Ok(SyscallReturn::Return(events_len as _))
}

pub fn sys_epoll_pwait2(
    epfd: FileDesc,
    events_addr: Vaddr,
    max_events: i32,
    timeout_addr: Vaddr,
    sigmask: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "epfd = {}, events_addr = 0x{:x}, max_events = {}, timeout_ts = 0x{:x}, sigmask = 0x{:x}",
        epfd, events_addr, max_events, timeout_addr, sigmask,
    );

    let timeout: Option<Duration> = if timeout_addr == 0 {
        None
    } else {
        let ts: timespec_t = ctx.user_space().read_val(timeout_addr)?;
        let duration = Duration::try_from(ts)?;
        Some(duration)
    };

    let events_len = do_epoll_pwait2(epfd, events_addr, max_events, timeout, sigmask, 8, ctx)?;

    Ok(SyscallReturn::Return(events_len as _))
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
