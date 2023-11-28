use crate::fs::file_table::FileDescripter;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_WRITE;
use crate::util::read_bytes_from_user;

use super::SyscallReturn;

const STDOUT: u64 = 1;
const STDERR: u64 = 2;

pub fn sys_write(
    fd: FileDescripter,
    user_buf_ptr: Vaddr,
    user_buf_len: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_WRITE);
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        fd, user_buf_ptr, user_buf_len
    );

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    if user_buf_len == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let mut buffer = vec![0u8; user_buf_len];
    read_bytes_from_user(user_buf_ptr, &mut buffer)?;
    debug!("write content = {:?}", buffer);
    let write_len = file.write(&buffer)?;
    Ok(SyscallReturn::Return(write_len as _))
}
