// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_listen(sockfd: FileDesc, backlog: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, backlog = {backlog}");

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    socket.listen(backlog as usize)?;

    Ok(SyscallReturn::Return(0))
}
