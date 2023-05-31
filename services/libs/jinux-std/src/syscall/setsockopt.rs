use crate::log_syscall_entry;
use crate::net::socket::SockOptionName;
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
    if level != SockOptionLevel::SOL_SOCKET {
        return_errno_with_message!(Errno::ENOPROTOOPT, "Unsupported sockoption level");
    }
    let sock_option_name = SockOptionName::try_from(optname)?;
    if optval == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval is null pointer");
    }
    let mut sock_opt_val = vec![0u8; optlen];
    read_bytes_from_user(optval, &mut sock_opt_val)?;

    debug!("sockfd = {sockfd}, optname = {sock_option_name:?}, optval = {sock_opt_val:?}");
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table
        .get_file(sockfd)?
        .as_socket()
        .ok_or(Error::with_message(
            Errno::ENOTSOCK,
            "the file is not socket",
        ))?;
    socket.set_sock_option(sock_option_name, &sock_opt_val)?;
    Ok(SyscallReturn::Return(0))
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
/// Sock Opt level
enum SockOptionLevel {
    SOL_IP = 0,
    SOL_SOCKET = 1,
}
