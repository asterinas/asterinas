// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::file_table::FileDescripter,
    log_syscall_entry,
    net::socket::{SockOptionLevel, SockOptionName},
    prelude::*,
    syscall::SYS_SETSOCKOPT,
    util::{read_val_from_user, write_val_to_user},
};

use super::SyscallReturn;

pub fn sys_getsockopt(
    sockfd: FileDescripter,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSOCKOPT);
    let level = SockOptionLevel::try_from(level)?;
    let sock_option_name = SockOptionName::try_from(optname)?;
    if optval == 0 || optlen_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval or optlen_addr is null pointer");
    }
    let optlen: u32 = read_val_from_user(optlen_addr)?;
    debug!(
        "level = {level:?}, sockfd = {sockfd}, optname = {sock_option_name:?}, optlen = {optlen}"
    );
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table
        .get_file(sockfd)?
        .as_socket()
        .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not socket"))?;

    // FIXME: This is only a workaround. Writing zero means the socket does not have error.
    // The linux manual says that writing a non-zero value if there are errors. But what value is it?
    if sock_option_name == SockOptionName::SO_ERROR {
        assert!(optlen == 4);
        write_val_to_user(optval, &0i32)?;
    }

    // TODO: do real getsockopt

    Ok(SyscallReturn::Return(0))
}
