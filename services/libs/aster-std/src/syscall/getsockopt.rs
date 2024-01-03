// SPDX-License-Identifier: MPL-2.0

use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::{get_socket_from_fd, new_raw_socket_option, CSocketOptionLevel};
use crate::util::{read_val_from_user, write_val_to_user};

use super::{SyscallReturn, SYS_SETSOCKOPT};

pub fn sys_getsockopt(
    sockfd: FileDescripter,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSOCKOPT);
    let level = CSocketOptionLevel::try_from(level)?;
    if optval == 0 || optlen_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval or optlen_addr is null pointer");
    }
    let optlen: u32 = read_val_from_user(optlen_addr)?;
    debug!("level = {level:?}, sockfd = {sockfd}, optname = {optname:?}, optlen = {optlen}");

    let socket = get_socket_from_fd(sockfd)?;

    let mut raw_option = new_raw_socket_option(level, optname)?;

    debug!("raw option: {:?}", raw_option);

    socket.get_option(raw_option.as_sock_option_mut())?;

    let write_len = {
        let current = current!();
        let vmar = current.root_vmar();
        raw_option.write_to_user(vmar, optval, optlen)?
    };

    write_val_to_user(optlen_addr, &(write_len as u32))?;

    Ok(SyscallReturn::Return(0))
}
