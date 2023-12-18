use crate::log_syscall_entry;
use crate::net::socket::{SockOptionLevel, SockOptionName};
use crate::util::read_bytes_from_user;
use crate::{fs::file_table::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_SETSOCKOPT;

pub fn sys_setsockopt(
    sockfd: FileDescripter,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSOCKOPT);
    let level = SockOptionLevel::try_from(level)?;
    let sock_option_name = SockOptionName::try_from(optname)?;
    if optval == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval is null pointer");
    }
    let mut sock_opt_val = vec![0u8; optlen];
    read_bytes_from_user(optval, &mut sock_opt_val)?;

    debug!("level = {level:?}, sockfd = {sockfd}, optname = {sock_option_name:?}, optval = {sock_opt_val:?}");
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table
        .get_file(sockfd)?
        .as_socket()
        .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not socket"))?;
    // TODO: do setsockopt
    Ok(SyscallReturn::Return(0))
}
