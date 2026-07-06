// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::file::FileLike,
    io_uring::{
        c_types::IoUringSqe,
        io_context::IoUringContext,
        ops::{IoUringOp, check_rw_flags, completion_from_result, get_file},
        utils::Completion,
    },
    net::socket::util::SocketAddr,
    prelude::*,
    util::net as user_net,
};

pub(in crate::io_uring::ops) struct IoUringConnectRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    socket_addr: SocketAddr,
}

impl IoUringConnectRequest {
    pub(in crate::io_uring::ops) fn new(
        _context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        check_rw_flags(sqe)?;

        let addr_len = usize::try_from(sqe.off).map_err(|_| {
            Error::with_message(Errno::EINVAL, "the socket address length is invalid")
        })?;
        let socket_addr = user_net::read_socket_addr_from_user(sqe.addr as Vaddr, addr_len)?;

        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            socket_addr,
        })
    }
}

impl IoUringOp for IoUringConnectRequest {
    fn try_execute_nonblock(&self) -> Option<Completion> {
        if self.force_async {
            return None;
        }

        None
    }

    fn execute(&self) -> Completion {
        let result = (|| {
            let socket = self.file.as_socket_or_err()?;
            socket.connect(self.socket_addr.clone())?;
            Ok(0)
        })();

        completion_from_result(result, self.user_data)
    }
}
