// SPDX-License-Identifier: MPL-2.0

use crate::net::socket::SockShutdownCmd;
use crate::{fs::file_table::FileDescripter, log_syscall_entry, prelude::*};

use super::SyscallReturn;
use super::SYS_SHUTDOWN;

pub fn sys_shutdown(sockfd: FileDescripter, how: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SHUTDOWN);
    let shutdown_cmd = SockShutdownCmd::try_from(how)?;
    debug!("sockfd = {sockfd}, cmd = {shutdown_cmd:?}");
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table
        .get_file(sockfd)?
        .as_socket()
        .ok_or(Error::with_message(
            Errno::ENOTSOCK,
            "the file is not socket",
        ))?;
    socket.shutdown(shutdown_cmd)?;
    Ok(SyscallReturn::Return(0))
}
