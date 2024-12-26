// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, net::socket::SockShutdownCmd, prelude::*};

pub fn sys_shutdown(sockfd: FileDesc, how: i32, ctx: &Context) -> Result<SyscallReturn> {
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    socket.shutdown(shutdown_cmd)?;

    Ok(SyscallReturn::Return(0))
}
