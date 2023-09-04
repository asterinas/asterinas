use crate::log_syscall_entry;
use crate::util::write_bytes_to_user;
use crate::{fs::file_table::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_READ;

pub fn sys_read(fd: FileDescripter, user_buf_addr: Vaddr, buf_len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_READ);
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, buf_len = 0x{:x}",
        fd, user_buf_addr, buf_len
    );
    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let mut read_buf = vec![0u8; buf_len];
    let read_len = file.read(&mut read_buf)?;
    write_bytes_to_user(user_buf_addr, &read_buf)?;
    Ok(SyscallReturn::Return(read_len as _))
}
