use crate::fs::file::FileDescripter;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_WRITE;
use crate::util::read_bytes_from_user;

use super::SyscallReturn;

const STDOUT: u64 = 1;
const STDERR: u64 = 2;

pub fn sys_write(
    fd: FileDescripter,
    user_buf_ptr: Vaddr,
    user_buf_len: u64,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_WRITE);
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        fd, user_buf_ptr, user_buf_len
    );

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let mut buffer = vec![0u8; user_buf_len as usize];
    read_bytes_from_user(user_buf_ptr as usize, &mut buffer)?;
    let write_len = file.write(&buffer)?;
    Ok(SyscallReturn::Return(write_len as _))
}
