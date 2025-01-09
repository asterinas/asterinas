// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
    util::net::{new_orig_socket_option, CSocketOptionLevel},
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

    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let original_option = {
        let mut option = new_orig_socket_option(level, optname)?;
        option.read_from_user(optval, optlen)?;
        option
    };
    debug!("original option: {:?}", original_option);

    socket.set_option(original_option.as_sock_option())?;

    Ok(SyscallReturn::Return(0))
}
