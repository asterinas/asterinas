// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
};

pub fn sys_listen(sockfd: FileDesc, backlog: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, backlog = {backlog}");

    get_file_fast! { let (file_table, file) = sockfd @ ctx.thread_local };
    let socket = file.as_socket_or_err()?;

    socket.listen(backlog as usize)?;

    Ok(SyscallReturn::Return(0))
}
