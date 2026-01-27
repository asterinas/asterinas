// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use crate::{
    fs::file_table::FileDesc,
    net::socket::{Socket, util::SendRecvFlags},
    prelude::*,
    syscall::{SyscallReturn, sendmsg::send_one_message},
    util::net::CUserMsgHdr,
};

pub fn sys_sendmmsg(
    sockfd: FileDesc,
    mmsghdrs_addr: Vaddr,
    count: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = SendRecvFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid send recv flags"))?;

    debug!(
        "sockfd = {}, mmsghdrs = {:#x}, count = {}, flags = {:?}",
        sockfd, mmsghdrs_addr, count, flags
    );

    if !flags.is_empty() {
        warn!("sendmmsg flags {:?} are not supported", flags);
    }

    let file = {
        // Reading control messages may access the file table,
        // so we have to clone the file and drop the file table reference here.
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.unwrap().read();
        file_table_locked.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let mut sent_msgs = 0;
    match send_mmsg_hdrs(socket, mmsghdrs_addr, count, flags, &mut sent_msgs, ctx) {
        // Only return error if no packets are sent successfully.
        Err(e) if sent_msgs == 0 => Err(e),
        _ => Ok(SyscallReturn::Return(sent_msgs as _)),
    }
}

#[repr(C)]
#[padding_struct]
#[derive(Debug, Clone, Copy, Pod)]
struct CMmsgHdr {
    msg_hdr: CUserMsgHdr,
    msg_len: u32,
}

fn send_mmsg_hdrs(
    socket: &dyn Socket,
    mmsghdrs_addr: Vaddr,
    count: usize,
    flags: SendRecvFlags,
    sent_msgs: &mut usize,
    ctx: &Context,
) -> Result<()> {
    let user_space = ctx.user_space();

    for i in 0..count {
        let addr = mmsghdrs_addr + size_of::<CMmsgHdr>() * i;
        let mut mmsghdr = user_space.read_val::<CMmsgHdr>(addr)?;

        let sent_bytes = send_one_message(socket, &mmsghdr.msg_hdr, &user_space, flags)?;

        mmsghdr.msg_len = sent_bytes as u32;
        user_space.write_val(addr, &mmsghdr)?;

        *sent_msgs += 1;
    }

    Ok(())
}
