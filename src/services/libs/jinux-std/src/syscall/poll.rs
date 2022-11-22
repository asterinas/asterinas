use core::time::Duration;

use crate::fs::poll::{c_pollfd, PollFd};
use crate::memory::{read_val_from_user, write_val_to_user};
use crate::{fs::poll::c_nfds, prelude::*};

use super::SyscallReturn;
use super::SYS_POLL;

pub fn sys_poll(fds: Vaddr, nfds: c_nfds, timeout: i32) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_POLL]", SYS_POLL);

    let mut read_addr = fds;
    let mut pollfds = Vec::with_capacity(nfds as _);
    for _ in 0..nfds {
        let c_poll_fd = read_val_from_user::<c_pollfd>(read_addr)?;
        let poll_fd = PollFd::from(c_poll_fd);
        pollfds.push(poll_fd);
        // FIXME: do we need to respect align of c_pollfd here?
        read_addr += core::mem::size_of::<c_pollfd>();
    }
    let timeout = if timeout == 0 {
        None
    } else {
        Some(Duration::from_millis(timeout as _))
    };
    debug!(
        "poll_fds = {:?}, nfds = {}, timeout = {:?}",
        pollfds, nfds, timeout
    );
    let current = current!();
    // FIXME: respect timeout parameter
    let ready_files = current.poll_queue().wait_until(|| {
        let mut ready_files = 0;
        for pollfd in &mut pollfds {
            let file_table = current.file_table().lock();
            let file = file_table.get_file(pollfd.fd);
            match file {
                None => return Some(Err(Error::new(Errno::EBADF))),
                Some(file) => {
                    let file_events = file.poll();
                    let polled_events = pollfd.events.intersection(file_events);
                    if !polled_events.is_empty() {
                        ready_files += 1;
                        pollfd.revents |= polled_events;
                    }
                }
            }
        }
        if ready_files > 0 {
            return Some(Ok(ready_files));
        } else {
            return None;
        }
    })?;
    let mut write_addr = fds;
    for pollfd in pollfds {
        let c_poll_fd = c_pollfd::from(pollfd);
        write_val_to_user(write_addr, &c_poll_fd)?;
        // FIXME: do we need to respect align of c_pollfd here?
        write_addr += core::mem::size_of::<c_pollfd>();
    }
    Ok(SyscallReturn::Return(ready_files))
}
