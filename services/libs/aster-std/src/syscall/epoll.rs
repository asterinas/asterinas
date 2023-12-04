use core::time::Duration;

use crate::events::IoEvents;
use crate::fs::epoll::{EpollCtl, EpollEvent, EpollFile, EpollFlags};
use crate::fs::file_table::FileDescripter;
use crate::fs::utils::CreationFlags;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::{read_val_from_user, write_val_to_user};

use super::SyscallReturn;
use super::{SYS_EPOLL_CREATE1, SYS_EPOLL_CTL, SYS_EPOLL_WAIT};

pub fn sys_epoll_create(size: i32) -> Result<SyscallReturn> {
    if size <= 0 {
        return_errno_with_message!(Errno::EINVAL, "size is not positive");
    }
    sys_epoll_create1(0)
}

pub fn sys_epoll_create1(flags: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EPOLL_CREATE1);
    debug!("flags = 0x{:x}", flags);

    let close_on_exec = {
        let flags = CreationFlags::from_bits(flags)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
        if flags == CreationFlags::empty() {
            false
        } else if flags == CreationFlags::O_CLOEXEC {
            true
        } else {
            // Only O_CLOEXEC is valid
            return_errno_with_message!(Errno::EINVAL, "invalid flags");
        }
    };

    let current = current!();
    let epoll_file: Arc<EpollFile> = EpollFile::new();
    let mut file_table = current.file_table().lock();
    let fd = file_table.insert(epoll_file);
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_epoll_ctl(
    epfd: FileDescripter,
    op: i32,
    fd: FileDescripter,
    event_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EPOLL_CTL);
    debug!(
        "epfd = {}, op = {}, fd = {}, event_addr = 0x{:x}",
        epfd, op, fd, event_addr
    );

    const EPOLL_CTL_ADD: i32 = 1;
    const EPOLL_CTL_DEL: i32 = 2;
    const EPOLL_CTL_MOD: i32 = 3;

    let cmd = match op {
        EPOLL_CTL_ADD => {
            let c_epoll_event = read_val_from_user::<c_epoll_event>(event_addr)?;
            let event = EpollEvent::from(&c_epoll_event);
            let flags = EpollFlags::from_bits_truncate(c_epoll_event.events);
            EpollCtl::Add(fd, event, flags)
        }
        EPOLL_CTL_DEL => EpollCtl::Del(fd),
        EPOLL_CTL_MOD => {
            let c_epoll_event = read_val_from_user::<c_epoll_event>(event_addr)?;
            let event = EpollEvent::from(&c_epoll_event);
            let flags = EpollFlags::from_bits_truncate(c_epoll_event.events);
            EpollCtl::Mod(fd, event, flags)
        }
        _ => return_errno_with_message!(Errno::EINVAL, "invalid op"),
    };

    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(epfd)?.clone()
    };
    let epoll_file = file
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;
    epoll_file.control(&cmd)?;

    Ok(SyscallReturn::Return(0 as _))
}

pub fn sys_epoll_wait(
    epfd: FileDescripter,
    events_addr: Vaddr,
    max_events: i32,
    timeout: i32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EPOLL_WAIT);

    let max_events = {
        if max_events <= 0 {
            return_errno_with_message!(Errno::EINVAL, "max_events is not positive");
        }
        max_events as usize
    };
    let timeout = if timeout >= 0 {
        Some(Duration::from_millis(timeout as _))
    } else {
        None
    };
    debug!(
        "epfd = {}, events_addr = 0x{:x}, max_events = {}, timeout = {:?}",
        epfd, events_addr, max_events, timeout
    );

    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(epfd)?.clone()
    };
    let epoll_file = file
        .downcast_ref::<EpollFile>()
        .ok_or(Error::with_message(Errno::EINVAL, "not epoll file"))?;
    let epoll_events = epoll_file.wait(max_events, timeout.as_ref())?;

    // Write back
    let mut write_addr = events_addr;
    for epoll_event in epoll_events.iter() {
        let c_epoll_event = c_epoll_event::from(epoll_event);
        write_val_to_user(write_addr, &c_epoll_event)?;
        write_addr += core::mem::size_of::<c_epoll_event>();
    }

    Ok(SyscallReturn::Return(epoll_events.len() as _))
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
