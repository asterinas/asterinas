// SPDX-License-Identifier: MPL-2.0

use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::get_socket_from_fd;

use super::{SyscallReturn, SYS_LISTEN};

pub fn sys_listen(sockfd: FileDescripter, backlog: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LISTEN);
    debug!("sockfd = {sockfd}, backlog = {backlog}");

    let socket = get_socket_from_fd(sockfd)?;

    socket.listen(backlog as usize)?;
    Ok(SyscallReturn::Return(0))
}
