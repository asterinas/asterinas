// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::SockShutdownCmd,
    prelude::*,
};

pub fn sys_shutdown(sockfd: FileDesc, how: i32, ctx: &Context) -> Result<SyscallReturn> {
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    get_file_fast! { let (file_table, file) = sockfd @ ctx.thread_local };
    let socket = file.as_socket_or_err()?;

    socket.shutdown(shutdown_cmd)?;

    Ok(SyscallReturn::Return(0))
}
