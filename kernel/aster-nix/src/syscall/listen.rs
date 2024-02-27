// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_LISTEN};
use crate::{
    fs::file_table::FileDescripter, log_syscall_entry, prelude::*, util::net::get_socket_from_fd,
};

pub fn sys_listen(sockfd: FileDescripter, backlog: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LISTEN);
    debug!("sockfd = {sockfd}, backlog = {backlog}");

    let socket = get_socket_from_fd(sockfd)?;

    socket.listen(backlog as usize)?;
    Ok(SyscallReturn::Return(0))
}
