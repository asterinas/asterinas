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
    net::socket::util::SendRecvFlags,
    prelude::*,
    util::net::CUserMsgHdr,
};

pub(in crate::io_uring::ops) struct IoUringRecvMsgRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    msghdr_addr: Vaddr,
    c_user_msghdr: CUserMsgHdr,
    flags: SendRecvFlags,
}

impl IoUringRecvMsgRequest {
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
                "io_uring recvmsg control messages are unsupported"
            );
        }

        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            msghdr_addr: sqe.addr as Vaddr,
            c_user_msghdr,
            flags: SendRecvFlags::from_bits_truncate(sqe.op_flags as i32),
        })
    }
}

impl IoUringOp for IoUringRecvMsgRequest {
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
            let mut writer = self
                .c_user_msghdr
                .copy_writer_array_from_user(&user_space)?;
            let (len, message_header) = socket.recvmsg(&mut writer, self.flags)?;

            let mut c_user_msghdr = self.c_user_msghdr;
            c_user_msghdr.msg_namelen =
                c_user_msghdr.write_socket_addr_to_user(message_header.addr())?;
            c_user_msghdr.msg_controllen = 0;
            user_space.write_val(self.msghdr_addr, &c_user_msghdr)?;

            Ok(len)
        })();

        completion_from_result(result, self.user_data)
    }
}
