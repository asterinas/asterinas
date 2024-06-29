// SPDX-License-Identifier: MPL-2.0

use super::{select::sys_select, SyscallReturn};
use crate::{fs::file_table::FileDesc, prelude::*};

pub fn sys_pselect6(
    nfds: FileDesc,
    readfds_addr: Vaddr,
    writefds_addr: Vaddr,
    exceptfds_addr: Vaddr,
    timeval_addr: Vaddr,
    sigmask_addr: Vaddr,
) -> Result<SyscallReturn> {
    // TODO: Support signal mask
    if sigmask_addr != 0 {
        error!("[SYS_PSELECT6] Not support sigmask now");
        return Err(Error::new(Errno::ENOSYS));
    }

    sys_select(
        nfds,
        readfds_addr,
        writefds_addr,
        exceptfds_addr,
        timeval_addr,
    )
}
