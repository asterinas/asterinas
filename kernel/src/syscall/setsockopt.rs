// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    util::net::{new_raw_socket_option, CSocketOptionLevel},
};

pub fn sys_setsockopt(
    sockfd: FileDesc,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let level = CSocketOptionLevel::try_from(level).map_err(|_| Errno::EOPNOTSUPP)?;
    if optval == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval is null pointer");
    }

    debug!(
        "level = {:?}, sockfd = {}, optname = {}, optval = {}",
        level, sockfd, optname, optlen
    );

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let raw_option = {
        let mut option = new_raw_socket_option(level, optname)?;
        option.read_from_user(optval, optlen)?;
        option
    };
    debug!("raw option: {:?}", raw_option);

    socket.set_option(raw_option.as_sock_option())?;

    Ok(SyscallReturn::Return(0))
}
