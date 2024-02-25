// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SHUTDOWN};
use crate::{
    fs::file_table::FileDescripter, log_syscall_entry, net::socket::SockShutdownCmd, prelude::*,
    util::net::get_socket_from_fd,
};

pub fn sys_shutdown(sockfd: FileDescripter, how: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SHUTDOWN);
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    let socket = get_socket_from_fd(sockfd)?;
    socket.shutdown(shutdown_cmd)?;
    Ok(SyscallReturn::Return(0))
}
