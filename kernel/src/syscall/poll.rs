// SPDX-License-Identifier: MPL-2.0

use core::{cell::Cell, time::Duration};

use super::SyscallReturn;
use crate::{events::IoEvents, fs::file_table::FileDesc, prelude::*, process::signal::Poller};

pub fn sys_poll(fds: Vaddr, nfds: u64, timeout: i32, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.get_user_space();
    let poll_fds = {
        let mut read_addr = fds;
        let mut poll_fds = Vec::with_capacity(nfds as _);
        for _ in 0..nfds {
            let c_poll_fd = user_space.read_val::<c_pollfd>(read_addr)?;
            let poll_fd = PollFd::from(c_poll_fd);
            // Always clear the revents fields first
            poll_fd.revents().set(IoEvents::empty());
            poll_fds.push(poll_fd);
            // FIXME: do we need to respect align of c_pollfd here?
            read_addr += core::mem::size_of::<c_pollfd>();
        }
        poll_fds
    };
    let timeout = if timeout >= 0 {
        Some(Duration::from_millis(timeout as _))
    } else {
        None
    };
    debug!(
        "poll_fds = {:?}, nfds = {}, timeout = {:?}",
        poll_fds, nfds, timeout
    );

    let num_revents = do_poll(&poll_fds, timeout, ctx)?;

    // Write back
    let mut write_addr = fds;
    for pollfd in poll_fds {
        let c_poll_fd = c_pollfd::from(pollfd);
        user_space.write_val(write_addr, &c_poll_fd)?;
        // FIXME: do we need to respect align of c_pollfd here?
        write_addr += core::mem::size_of::<c_pollfd>();
    }

    Ok(SyscallReturn::Return(num_revents as _))
}

pub fn do_poll(poll_fds: &[PollFd], timeout: Option<Duration>, ctx: &Context) -> Result<usize> {
    // The main loop of polling
    let mut poller = Poller::new();
    loop {
        let mut num_revents = 0;

        let file_table = ctx.process.file_table().lock();
        for poll_fd in poll_fds {
            // Skip poll_fd if it is not given a fd
            let fd = match poll_fd.fd() {
                Some(fd) => fd,
                None => continue,
            };

            // Poll the file
            let file = file_table.get_file(fd)?;
            let need_poller = if num_revents == 0 {
                Some(&mut poller)
            } else {
                None
            };
            let revents = file.poll(poll_fd.events(), need_poller);
            if !revents.is_empty() {
                poll_fd.revents().set(revents);
                num_revents += 1;
            }
        }

        drop(file_table);

        if num_revents > 0 {
            return Ok(num_revents);
        }

        // Return immediately if specifying a timeout of zero
        if timeout.is_some() && timeout.as_ref().unwrap().is_zero() {
            return Ok(0);
        }

        if let Some(timeout) = timeout.as_ref() {
            match poller.wait_timeout(timeout) {
                Ok(_) => {}
                Err(e) if e.error() == Errno::ETIME => {
                    // The return value is zero if the timeout expires
                    // before any file descriptors became ready
                    return Ok(0);
                }
                Err(e) => return Err(e),
            };
        } else {
            poller.wait()?;
        }
    }
}

// https://github.com/torvalds/linux/blob/master/include/uapi/asm-generic/poll.h
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct c_pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[derive(Debug, Clone)]
pub struct PollFd {
    fd: Option<FileDesc>,
    events: IoEvents,
    revents: Cell<IoEvents>,
}

impl PollFd {
    pub fn new(fd: Option<FileDesc>, events: IoEvents) -> Self {
        let revents = Cell::new(IoEvents::empty());
        Self {
            fd,
            events,
            revents,
        }
    }

    pub fn fd(&self) -> Option<FileDesc> {
        self.fd
    }

    pub fn events(&self) -> IoEvents {
        self.events
    }

    pub fn revents(&self) -> &Cell<IoEvents> {
        &self.revents
    }
}

impl From<c_pollfd> for PollFd {
    fn from(raw: c_pollfd) -> Self {
        let fd = if raw.fd >= 0 {
            Some(raw.fd as FileDesc)
        } else {
            None
        };
        let events = IoEvents::from_bits_truncate(raw.events as _);
        let revents = Cell::new(IoEvents::from_bits_truncate(raw.revents as _));
        Self {
            fd,
            events,
            revents,
        }
    }
}

impl From<PollFd> for c_pollfd {
    fn from(raw: PollFd) -> Self {
        let fd = raw.fd().unwrap_or(-1);
        let events = raw.events().bits() as i16;
        let revents = raw.revents().get().bits() as i16;
        Self {
            fd,
            events,
            revents,
        }
    }
}
