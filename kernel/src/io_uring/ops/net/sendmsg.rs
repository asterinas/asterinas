// SPDX-License-Identifier: MPL-2.0

use ostd::{mm::VmIo, task::Task};

use crate::{
    fs::file::FileLike,
    io_uring::{
        c_types::IoUringSqe,
        io_context::IoUringContext,
        ops::{IoUringOp, completion_from_result, get_file},
        utils::Completion,
    },
    net::socket::util::{MessageHeader, SendRecvFlags, SocketAddr},
    prelude::*,
    util::net::CUserMsgHdr,
};

pub(in crate::io_uring::ops) struct IoUringSendMsgRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    c_user_msghdr: CUserMsgHdr,
    addr: Option<SocketAddr>,
    flags: SendRecvFlags,
}

impl IoUringSendMsgRequest {
    pub(in crate::io_uring::ops) fn new(
        _context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        let task = Task::current().unwrap();
        let thread_local = task.as_thread_local().unwrap();
        let user_space = CurrentUserSpace::new(thread_local);
        let c_user_msghdr = user_space.read_val::<CUserMsgHdr>(sqe.addr as Vaddr)?;
        if c_user_msghdr.msg_control != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "io_uring sendmsg control messages are unsupported"
            );
        }
        let addr = c_user_msghdr.read_socket_addr_from_user()?;

        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            c_user_msghdr,
            addr,
            flags: SendRecvFlags::from_bits_truncate(sqe.op_flags as i32),
        })
    }
}

impl IoUringOp for IoUringSendMsgRequest {
    fn try_execute_nonblock(&self) -> Option<Completion> {
        if self.force_async {
            return None;
        }

        None
    }

    fn execute(&self) -> Completion {
        let result = (|| {
            let socket = self.file.as_socket_or_err()?;
            let task = Task::current().unwrap();
            let thread_local = task.as_thread_local().unwrap();
            let user_space = CurrentUserSpace::new(thread_local);
            let mut reader = self
                .c_user_msghdr
                .copy_reader_array_from_user(&user_space)?;

            socket.sendmsg(
                &mut reader,
                MessageHeader::new(self.addr.clone(), Vec::new()),
                self.flags,
            )
        })();

        completion_from_result(result, self.user_data)
    }
}
