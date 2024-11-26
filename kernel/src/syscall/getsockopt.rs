// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    util::net::{get_socket_from_fd, new_raw_socket_option, CSocketOptionLevel},
};

pub fn sys_getsockopt(
    sockfd: FileDesc,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let level = CSocketOptionLevel::try_from(level)?;
    if optval == 0 || optlen_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval or optlen_addr is null pointer");
    }
    let user_space = ctx.user_space();

    let optlen: u32 = user_space.read_val(optlen_addr)?;
    debug!("level = {level:?}, sockfd = {sockfd}, optname = {optname:?}, optlen = {optlen}");

    let socket = get_socket_from_fd(sockfd)?;

    let mut raw_option = new_raw_socket_option(level, optname)?;

    debug!("raw option: {:?}", raw_option);

    socket.get_option(raw_option.as_sock_option_mut())?;

    let write_len = raw_option.write_to_user(optval, optlen)?;

    user_space.write_val(optlen_addr, &(write_len as u32))?;

    Ok(SyscallReturn::Return(0))
}
