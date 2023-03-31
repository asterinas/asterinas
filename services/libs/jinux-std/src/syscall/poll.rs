use core::time::Duration;

use crate::fs::utils::{c_pollfd, IoEvents, PollFd, Poller};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::{read_val_from_user, write_val_to_user};

use super::SyscallReturn;
use super::SYS_POLL;

pub fn sys_poll(fds: Vaddr, nfds: u64, timeout: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_POLL);

    let poll_fds = {
        let mut read_addr = fds;
        let mut poll_fds = Vec::with_capacity(nfds as _);
        for _ in 0..nfds {
            let c_poll_fd = read_val_from_user::<c_pollfd>(read_addr)?;
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

    let num_revents = do_poll(&poll_fds, timeout)?;

    // Write back
    let mut write_addr = fds;
    for pollfd in poll_fds {
        let c_poll_fd = c_pollfd::from(pollfd);
        write_val_to_user(write_addr, &c_poll_fd)?;
        // FIXME: do we need to respect align of c_pollfd here?
        write_addr += core::mem::size_of::<c_pollfd>();
    }

    Ok(SyscallReturn::Return(num_revents as _))
}

fn do_poll(poll_fds: &[PollFd], timeout: Option<Duration>) -> Result<usize> {
    // The main loop of polling
    let poller = Poller::new();
    loop {
        let mut num_revents = 0;

        for poll_fd in poll_fds {
            // Skip poll_fd if it is not given a fd
            let fd = match poll_fd.fd() {
                Some(fd) => fd,
                None => continue,
            };

            // Poll the file
            let current = current!();
            let file = {
                let file_table = current.file_table().lock();
                file_table.get_file(fd)?.clone()
            };
            let need_poller = if num_revents == 0 {
                Some(&poller)
            } else {
                None
            };
            let revents = file.poll(poll_fd.events(), need_poller);
            if !revents.is_empty() {
                poll_fd.revents().set(revents);
                num_revents += 1;
            }
        }

        if num_revents > 0 {
            return Ok(num_revents);
        }

        // Return immediately if specifying a timeout of zero
        if timeout.is_some() && timeout.as_ref().unwrap().is_zero() {
            return Ok(0);
        }

        // FIXME: respect timeout parameter
        poller.wait();
    }
}
