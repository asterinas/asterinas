use super::SYS_STAT;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;
use crate::{log_syscall_entry, prelude::*};

use super::SyscallReturn;

pub fn sys_stat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_STAT);
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    debug!(
        "filename = {:?}, stat_buf_ptr = 0x{:x}",
        filename, stat_buf_ptr
    );
    return_errno_with_message!(Errno::ENOSYS, "Stat is unimplemented");
}
