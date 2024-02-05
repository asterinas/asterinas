// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use crate::events::IoEvents;
use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::time::timeval_t;
use crate::util::{read_val_from_user, write_val_to_user};

use super::poll::{do_poll, PollFd};
use super::SyscallReturn;
use super::SYS_SELECT;

pub fn sys_select(
    nfds: FileDescripter,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timeval_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SELECT);

    if nfds < 0 || nfds as usize > FD_SETSIZE {
        return_errno_with_message!(Errno::EINVAL, "nfds is negative or exceeds the FD_SETSIZE");
    }

    let get_fdset = |fdset_addr: Vaddr| -> Result<Option<FdSet>> {
        let fdset = if fdset_addr == 0 {
            None
        } else {
            let fdset = read_val_from_user::<FdSet>(fdset_addr)?;
            Some(fdset)
        };
        Ok(fdset)
    };
    let mut readfds = get_fdset(readfds_addr)?;
    let mut writefds = get_fdset(writefds_addr)?;
    let mut exceptfds = get_fdset(exceptfds_addr)?;

    let timeout = if timeval_addr == 0 {
        None
    } else {
        let timeval = read_val_from_user::<timeval_t>(timeval_addr)?;
        Some(Duration::from(timeval))
    };

    debug!(
        "nfds = {}, readfds = {:?}, writefds = {:?}, exceptfds = {:?}, timeout = {:?}",
        nfds, readfds, writefds, exceptfds, timeout
    );

    let num_revents = do_select(
        nfds,
        readfds.as_mut(),
        writefds.as_mut(),
        exceptfds.as_mut(),
        timeout,
    )?;

    let set_fdset = |fdset_addr: Vaddr, fdset: Option<FdSet>| -> Result<()> {
        if let Some(fdset) = fdset {
            debug_assert!(fdset_addr != 0);
            write_val_to_user(fdset_addr, &fdset)?;
        }
        Ok(())
    };
    set_fdset(readfds_addr, readfds)?;
    set_fdset(writefds_addr, writefds)?;
    set_fdset(exceptfds_addr, exceptfds)?;

    Ok(SyscallReturn::Return(num_revents as _))
}

fn do_select(
    nfds: FileDescripter,
    mut readfds: Option<&mut FdSet>,
    mut writefds: Option<&mut FdSet>,
    mut exceptfds: Option<&mut FdSet>,
    timeout: Option<Duration>,
) -> Result<usize> {
    // Convert the FdSet to an array of PollFd
    let poll_fds = {
        let mut poll_fds = Vec::new();
        for fd in 0..nfds {
            let events = {
                let readable = readfds.as_ref().map_or(false, |fds| fds.is_set(fd));
                let writable = writefds.as_ref().map_or(false, |fds| fds.is_set(fd));
                let except = exceptfds.as_ref().map_or(false, |fds| fds.is_set(fd));
                convert_rwe_to_events(readable, writable, except)
            };

            if events.is_empty() {
                continue;
            }

            let poll_fd = PollFd::new(Some(fd), events);
            poll_fds.push(poll_fd);
        }
        poll_fds
    };

    // Clear up the three input fd_set's, which will be used for output as well
    if let Some(fds) = readfds.as_mut() {
        fds.clear();
    }
    if let Some(fds) = writefds.as_mut() {
        fds.clear();
    }
    if let Some(fds) = exceptfds.as_mut() {
        fds.clear();
    }

    // Do the poll syscall that is equivalent to the select syscall
    let num_revents = do_poll(&poll_fds, timeout)?;
    if num_revents == 0 {
        return Ok(0);
    }

    // Convert poll's pollfd results to select's fd_set results
    let mut total_revents = 0;
    for poll_fd in &poll_fds {
        let fd = poll_fd.fd().unwrap();
        let revents = poll_fd.revents().get();
        let (readable, writable, except) = convert_events_to_rwe(&revents);
        if let Some(ref mut fds) = readfds
            && readable
        {
            fds.set(fd)?;
            total_revents += 1;
        }
        if let Some(ref mut fds) = writefds
            && writable
        {
            fds.set(fd)?;
            total_revents += 1;
        }
        if let Some(ref mut fds) = exceptfds
            && except
        {
            fds.set(fd)?;
            total_revents += 1;
        }
    }
    Ok(total_revents)
}

// Convert select's rwe input to poll's IoEvents input according to Linux's
// behavior.
fn convert_rwe_to_events(readable: bool, writable: bool, except: bool) -> IoEvents {
    let mut events = IoEvents::empty();
    if readable {
        events |= IoEvents::IN;
    }
    if writable {
        events |= IoEvents::OUT;
    }
    if except {
        events |= IoEvents::PRI;
    }
    events
}

// Convert poll's IoEvents results to select's rwe results according to Linux's
// behavior.
fn convert_events_to_rwe(events: &IoEvents) -> (bool, bool, bool) {
    let readable = events.intersects(IoEvents::IN | IoEvents::HUP | IoEvents::ERR);
    let writable = events.intersects(IoEvents::OUT | IoEvents::ERR);
    let except = events.contains(IoEvents::PRI);
    (readable, writable, except)
}

const FD_SETSIZE: usize = 1024;
const USIZE_BITS: usize = core::mem::size_of::<usize>() * 8;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct FdSet {
    fds_bits: [usize; FD_SETSIZE / USIZE_BITS],
}

impl FdSet {
    /// Equivalent to FD_SET.
    pub fn set(&mut self, fd: FileDescripter) -> Result<()> {
        let fd = fd as usize;
        if fd >= FD_SETSIZE {
            return_errno_with_message!(Errno::EINVAL, "fd exceeds FD_SETSIZE");
        }
        self.fds_bits[fd / USIZE_BITS] |= 1 << (fd % USIZE_BITS);
        Ok(())
    }

    /// Equivalent to FD_CLR.
    pub fn unset(&mut self, fd: FileDescripter) -> Result<()> {
        let fd = fd as usize;
        if fd >= FD_SETSIZE {
            return_errno_with_message!(Errno::EINVAL, "fd exceeds FD_SETSIZE");
        }
        self.fds_bits[fd / USIZE_BITS] &= !(1 << (fd % USIZE_BITS));
        Ok(())
    }

    /// Equivalent to FD_ISSET.
    pub fn is_set(&self, fd: FileDescripter) -> bool {
        let fd = fd as usize;
        if fd >= FD_SETSIZE {
            return false;
        }
        (self.fds_bits[fd / USIZE_BITS] & (1 << (fd % USIZE_BITS))) != 0
    }

    /// Equivalent to FD_ZERO.
    pub fn clear(&mut self) {
        for slot in self.fds_bits.iter_mut() {
            *slot = 0;
        }
    }
}
