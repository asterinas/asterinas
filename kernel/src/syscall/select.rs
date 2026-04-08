// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::mm::VmIo;

use super::{
    SyscallReturn,
    poll::{PollFd, do_poll},
};
use crate::{
    events::IoEvents,
    fs::file::file_table::{FileDesc, RawFileDesc},
    prelude::*,
    time::timeval_t,
};

pub fn sys_select(
    nfds: RawFileDesc,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timeval_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let timeout = if timeval_addr == 0 {
        None
    } else {
        let timeval = ctx
            .user_space()
            .read_val::<timeval_t>(timeval_addr)?
            .normalize();
        Some(Duration::try_from(timeval)?)
    };

    do_sys_select(
        nfds,
        readfds_addr,
        writefds_addr,
        exceptfds_addr,
        timeout,
        ctx,
    )
}

pub(super) fn do_sys_select(
    nfds: RawFileDesc,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timeout: Option<Duration>,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if nfds < 0 || nfds as usize > FD_SETSIZE {
        return_errno_with_message!(Errno::EINVAL, "nfds is negative or exceeds the FD_SETSIZE");
    }

    let user_space = ctx.user_space();
    let get_fdset = |fdset_addr: Vaddr| -> Result<Option<FdSet>> {
        let fdset = if fdset_addr == 0 {
            None
        } else {
            let fdset = user_space.read_val::<FdSet>(fdset_addr)?;
            Some(fdset)
        };
        Ok(fdset)
    };
    let mut readfds = get_fdset(readfds_addr)?;
    let mut writefds = get_fdset(writefds_addr)?;
    let mut exceptfds = get_fdset(exceptfds_addr)?;

    debug!(
        "nfds = {}, readfds = {:?}, writefds = {:?}, exceptfds = {:?}, timeout = {:?}",
        nfds, readfds, writefds, exceptfds, timeout
    );

    let num_revents = do_select(
        nfds,
        readfds.as_mut(),
        writefds.as_mut(),
        exceptfds.as_mut(),
        timeout.as_ref(),
        ctx,
    )?;

    // FIXME: The Linux select() and pselect6() system call
    // modifies its timeout argument to reflect the amount of time not slept.
    // However, the glibc wrapper function hides this behavior.
    // Maybe we should follow the Linux behavior.

    let set_fdset = |fdset_addr: Vaddr, fdset: Option<FdSet>| -> Result<()> {
        if let Some(fdset) = fdset {
            debug_assert!(fdset_addr != 0);
            user_space.write_val(fdset_addr, &fdset)?;
        }
        Ok(())
    };
    set_fdset(readfds_addr, readfds)?;
    set_fdset(writefds_addr, writefds)?;
    set_fdset(exceptfds_addr, exceptfds)?;

    Ok(SyscallReturn::Return(num_revents as _))
}

fn do_select(
    nfds: RawFileDesc,
    mut readfds: Option<&mut FdSet>,
    mut writefds: Option<&mut FdSet>,
    mut exceptfds: Option<&mut FdSet>,
    timeout: Option<&Duration>,
    ctx: &Context,
) -> Result<usize> {
    // Convert the `FdSet`s to an array of `PollFd`s
    let poll_fds = {
        let mut poll_fds = Vec::with_capacity(nfds as usize);
        for raw_fd in 0..nfds {
            let events = {
                let readable = readfds.as_ref().is_some_and(|fds| fds.is_set(raw_fd));
                let writable = writefds.as_ref().is_some_and(|fds| fds.is_set(raw_fd));
                let except = exceptfds.as_ref().is_some_and(|fds| fds.is_set(raw_fd));
                convert_rwe_to_events(readable, writable, except)
            };

            if events.is_empty() {
                continue;
            }

            let Ok(fd) = FileDesc::try_from(raw_fd) else {
                // Linux will ignore FDs that are greater than the maximum FD ever opened by the
                // process. So here we can break the loop and skip the remaining FDs.
                break;
            };
            let poll_fd = PollFd::new(Some(fd), events);
            poll_fds.push(poll_fd);
        }
        poll_fds
    };

    // Clear up the three input `FdSet`s, which will be used for output as well
    if let Some(fds) = readfds.as_mut() {
        fds.clear();
    }
    if let Some(fds) = writefds.as_mut() {
        fds.clear();
    }
    if let Some(fds) = exceptfds.as_mut() {
        fds.clear();
    }

    // Do the `poll` syscall that is equivalent to the `select` syscall
    let num_revents = do_poll(&poll_fds, timeout, ctx)?;
    if num_revents == 0 {
        return Ok(0);
    }

    // Convert `PollFd` results to `FdSet` results
    let mut total_revents = 0;
    for poll_fd in &poll_fds {
        let raw_fd = RawFileDesc::from(poll_fd.fd().unwrap());
        let revents = poll_fd.revents().get();
        let (readable, writable, except) = convert_events_to_rwe(revents)?;
        if let Some(ref mut fds) = readfds
            && readable
        {
            fds.set(raw_fd)?;
            total_revents += 1;
        }
        if let Some(ref mut fds) = writefds
            && writable
        {
            fds.set(raw_fd)?;
            total_revents += 1;
        }
        if let Some(ref mut fds) = exceptfds
            && except
        {
            fds.set(raw_fd)?;
            total_revents += 1;
        }
    }
    Ok(total_revents)
}

/// Converts `select` RWE input to `poll` I/O event input
/// according to Linux's behavior.
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

/// Converts `poll` I/O event results to `select` RWE results
/// according to Linux's behavior.
fn convert_events_to_rwe(events: IoEvents) -> Result<(bool, bool, bool)> {
    if events.contains(IoEvents::NVAL) {
        return_errno_with_message!(Errno::EBADF, "the file descriptor is invalid");
    }

    let readable = events.intersects(IoEvents::IN | IoEvents::HUP | IoEvents::ERR);
    let writable = events.intersects(IoEvents::OUT | IoEvents::ERR);
    let except = events.contains(IoEvents::PRI);
    Ok((readable, writable, except))
}

const FD_SETSIZE: usize = 1024;
const USIZE_BITS: usize = usize::BITS as usize;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct FdSet {
    fds_bits: [usize; FD_SETSIZE / USIZE_BITS],
}

impl FdSet {
    /// Equivalent to FD_SET.
    pub fn set(&mut self, fd: RawFileDesc) -> Result<()> {
        let fd = fd as usize;
        if fd >= FD_SETSIZE {
            return_errno_with_message!(Errno::EINVAL, "fd exceeds FD_SETSIZE");
        }
        self.fds_bits[fd / USIZE_BITS] |= 1 << (fd % USIZE_BITS);
        Ok(())
    }

    /// Equivalent to FD_CLR.
    #[expect(unused)]
    pub fn unset(&mut self, fd: RawFileDesc) -> Result<()> {
        let fd = fd as usize;
        if fd >= FD_SETSIZE {
            return_errno_with_message!(Errno::EINVAL, "fd exceeds FD_SETSIZE");
        }
        self.fds_bits[fd / USIZE_BITS] &= !(1 << (fd % USIZE_BITS));
        Ok(())
    }

    /// Equivalent to FD_ISSET.
    pub fn is_set(&self, fd: RawFileDesc) -> bool {
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
