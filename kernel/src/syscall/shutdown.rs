// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc, net::socket::SockShutdownCmd, prelude::*,
    util::net::get_socket_from_fd,
};

pub fn sys_shutdown(sockfd: FileDesc, how: i32, _ctx: &Context) -> Result<SyscallReturn> {
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    let socket = get_socket_from_fd(sockfd)?;
    socket.shutdown(shutdown_cmd)?;
    Ok(SyscallReturn::Return(0))
}
