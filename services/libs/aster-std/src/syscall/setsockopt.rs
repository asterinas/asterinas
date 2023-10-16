use crate::util::net::{new_raw_socket_option, SockOptionLevel};
use crate::{fs::file_table::FileDescripter, prelude::*};
use crate::{get_socket_without_holding_filetable_lock, log_syscall_entry};

use super::{SyscallReturn, SYS_SETSOCKOPT};

pub fn sys_setsockopt(
    sockfd: FileDescripter,
    level: i32,
    optname: i32,
    optval: Vaddr,
    optlen: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETSOCKOPT);
    let level = SockOptionLevel::try_from(level)?;
    if optval == 0 {
        return_errno_with_message!(Errno::EINVAL, "optval is null pointer");
    }

    debug!(
        "level = {:?}, sockfd = {}, optname = {}, optval = {}",
        level, sockfd, optname, optlen
    );

    let current = current!();
    get_socket_without_holding_filetable_lock!(socket, current, sockfd);

    let raw_option = {
        let mut option = new_raw_socket_option(level, optname)?;

        let vmar = current.root_vmar();
        option.read_input(vmar, optval, optlen)?;

        option
    };

    debug!("raw option: {:?}", raw_option);

    socket.set_option(raw_option.as_sock_option())?;

    Ok(SyscallReturn::Return(0))
}
