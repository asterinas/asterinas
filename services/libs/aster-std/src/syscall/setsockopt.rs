use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::{get_socket_from_fd, new_raw_socket_option, CSocketOptionLevel};

use super::{SyscallReturn, SYS_SETSOCKOPT};

pub fn sys_setsockopt(
    sockfd: FileDescripter,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSOCKOPT);
    let level = CSocketOptionLevel::try_from(level)?;
    if optval == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval is null pointer");
    }

    debug!(
        "level = {:?}, sockfd = {}, optname = {}, optval = {}",
        level, sockfd, optname, optlen
    );

    let socket = get_socket_from_fd(sockfd)?;

    let raw_option = {
        let mut option = new_raw_socket_option(level, optname)?;

        let current = current!();
        let vmar = current.root_vmar();
        option.read_from_user(vmar, optval, optlen)?;

        option
    };

    debug!("raw option: {:?}", raw_option);

    socket.set_option(raw_option.as_sock_option())?;

    Ok(SyscallReturn::Return(0))
}
