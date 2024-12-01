// SPDX-License-Identifier: MPL-2.0

use core::{cell::Cell, time::Duration};

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{file_handle::FileLike, file_table::FileDesc},
    prelude::*,
    process::signal::Poller,
};

pub fn sys_poll(fds: Vaddr, nfds: u64, timeout: i32, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();

    let poll_fds = {
        let mut read_addr = fds;
        let mut poll_fds = Vec::with_capacity(nfds as _);

        for _ in 0..nfds {
            let c_poll_fd = user_space.read_val::<c_pollfd>(read_addr)?;
            read_addr += core::mem::size_of::<c_pollfd>();

            let poll_fd = PollFd::from(c_poll_fd);
            // Always clear the revents fields first
            poll_fd.revents().set(IoEvents::empty());
            poll_fds.push(poll_fd);
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

    let num_revents = do_poll(&poll_fds, timeout.as_ref(), ctx)?;

    // Write back
    let mut write_addr = fds;
    for pollfd in poll_fds {
        let c_poll_fd = c_pollfd::from(pollfd);

        user_space.write_val(write_addr, &c_poll_fd)?;
        write_addr += core::mem::size_of::<c_pollfd>();
    }

    Ok(SyscallReturn::Return(num_revents as _))
}

pub fn do_poll(poll_fds: &[PollFd], timeout: Option<&Duration>, ctx: &Context) -> Result<usize> {
    let (result, files) = hold_files(poll_fds, ctx);
    match result {
        FileResult::AllValid => (),
        FileResult::SomeInvalid => {
            return Ok(count_all_events(poll_fds, &files));
        }
    }

    let poller = match register_poller(poll_fds, files.as_ref()) {
        PollerResult::AllRegistered(poller) => poller,
        PollerResult::EventFoundAt(index) => {
            let next = index + 1;
            let remaining_events = count_all_events(&poll_fds[next..], &files[next..]);
            return Ok(1 + remaining_events);
        }
    };

    loop {
        match poller.wait(timeout) {
            Ok(_) => {}
            Err(e) if e.error() == Errno::ETIME => {
                // The return value is zero if the timeout expires
                // before any file descriptors became ready
                return Ok(0);
            }
            Err(e) => return Err(e),
        };

        let num_events = count_all_events(poll_fds, &files);
        if num_events > 0 {
            return Ok(num_events);
        }

        // FIXME: We need to update `timeout` since we have waited for some time.
    }
}

enum FileResult {
    AllValid,
    SomeInvalid,
}

/// Holds all the files we're going to poll.
fn hold_files(poll_fds: &[PollFd], ctx: &Context) -> (FileResult, Vec<Option<Arc<dyn FileLike>>>) {
    let file_table = ctx.posix_thread.file_table().lock();

    let mut files = Vec::with_capacity(poll_fds.len());
    let mut result = FileResult::AllValid;

    for poll_fd in poll_fds.iter() {
        let Some(fd) = poll_fd.fd() else {
            files.push(None);
            continue;
        };

        let Ok(file) = file_table.get_file(fd) else {
            poll_fd.revents.set(IoEvents::NVAL);
            result = FileResult::SomeInvalid;

            files.push(None);
            continue;
        };

        files.push(Some(file.clone()));
    }

    (result, files)
}

enum PollerResult {
    AllRegistered(Poller),
    EventFoundAt(usize),
}

/// Registers the files with a poller, or exits early if some events are detected.
fn register_poller(poll_fds: &[PollFd], files: &[Option<Arc<dyn FileLike>>]) -> PollerResult {
    let mut poller = Poller::new();

    for (i, (poll_fd, file)) in poll_fds.iter().zip(files.iter()).enumerate() {
        let Some(file) = file else {
            continue;
        };

        let events = file.poll(poll_fd.events(), Some(poller.as_handle_mut()));
        if events.is_empty() {
            continue;
        }

        poll_fd.revents().set(events);
        return PollerResult::EventFoundAt(i);
    }

    PollerResult::AllRegistered(poller)
}

/// Counts the number of the ready files.
fn count_all_events(poll_fds: &[PollFd], files: &[Option<Arc<dyn FileLike>>]) -> usize {
    let mut counter = 0;

    for (poll_fd, file) in poll_fds.iter().zip(files.iter()) {
        let Some(file) = file else {
            if !poll_fd.revents.get().is_empty() {
                // This is only possible for POLLNVAL.
                counter += 1;
            }
            continue;
        };

        let events = file.poll(poll_fd.events(), None);
        if events.is_empty() {
            continue;
        }

        poll_fd.revents().set(events);
        counter += 1;
    }

    counter
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
