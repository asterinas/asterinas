// SPDX-License-Identifier: MPL-2.0

use crate::{fs::file_table::FileDescripter, log_syscall_entry, prelude::*};

use super::SyscallReturn;
use super::SYS_LISTEN;

pub fn sys_listen(sockfd: FileDescripter, backlog: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LISTEN);
    debug!("sockfd = {sockfd}, backlog = {backlog}");
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table.get_socket(sockfd)?;
    socket.listen(backlog as usize)?;
    Ok(SyscallReturn::Return(0))
}
