// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::util::SockShutdownCmd,
    prelude::*,
};

pub fn sys_shutdown(sockfd: FileDesc, how: i32, ctx: &Context) -> Result<SyscallReturn> {
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    socket.shutdown(shutdown_cmd)?;

    Ok(SyscallReturn::Return(0))
}
