// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*, util::net::get_socket_from_fd};

pub fn sys_listen(sockfd: FileDesc, backlog: i32, _ctx: &Context) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, backlog = {backlog}");

    let socket = get_socket_from_fd(sockfd)?;

    socket.listen(backlog as usize)?;
    Ok(SyscallReturn::Return(0))
}
