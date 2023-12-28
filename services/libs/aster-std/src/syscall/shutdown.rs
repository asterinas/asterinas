use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::net::socket::SockShutdownCmd;
use crate::prelude::*;
use crate::util::net::get_socket_from_fd;

use super::{SyscallReturn, SYS_SHUTDOWN};

pub fn sys_shutdown(sockfd: FileDescripter, how: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SHUTDOWN);
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");

    let socket = get_socket_from_fd(sockfd)?;
    socket.shutdown(shutdown_cmd)?;
    Ok(SyscallReturn::Return(0))
}
