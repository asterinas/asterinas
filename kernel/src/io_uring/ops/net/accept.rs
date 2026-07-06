// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    fs::file::{FileLike, StatusFlags},
    io_uring::{
        c_types::IoUringSqe,
        io_context::IoUringContext,
        ops::{IoUringOp, completion_from_result, get_file},
        utils::Completion,
    },
    net::socket::util::AcceptFlags,
    prelude::*,
    util::net as user_net,
};

pub(in crate::io_uring::ops) struct IoUringAcceptRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: AcceptFlags,
}

impl IoUringAcceptRequest {
    pub(in crate::io_uring::ops) fn new(
        _context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            sockaddr_ptr: sqe.addr as Vaddr,
            addrlen_ptr: sqe.off as Vaddr,
            flags: AcceptFlags::from_bits(sqe.rw_flags)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid accept flags"))?,
        })
    }
}

impl IoUringOp for IoUringAcceptRequest {
    fn try_execute_nonblock(&self) -> Option<Completion> {
        if self.force_async {
            return None;
        }

        None
    }

    fn execute(&self) -> Completion {
        let result = (|| {
            let socket = self.file.as_socket_or_err()?;
            let (connected_socket, socket_addr) = socket.accept()?;

            if self.flags.is_nonblocking() {
                connected_socket.set_status_flags(StatusFlags::O_NONBLOCK)?;
            }

            if self.sockaddr_ptr != 0 {
                user_net::write_socket_addr_to_user(
                    &socket_addr,
                    self.sockaddr_ptr,
                    self.addrlen_ptr,
                )?;
            }

            let task = Task::current().unwrap();
            let thread_local = task.as_thread_local().unwrap();
            let file_table = thread_local.borrow_file_table();
            let fd = file_table
                .unwrap()
                .write()
                .insert(connected_socket, self.flags.fd_flags());
            Ok(fd.into())
        })();

        completion_from_result(result, self.user_data)
    }
}
