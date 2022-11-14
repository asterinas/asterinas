use crate::prelude::*;

use crate::{memory::read_bytes_from_user, syscall::SYS_WRITE};

use super::SyscallReturn;

const STDOUT: u64 = 1;
const STDERR: u64 = 2;

pub fn sys_write(fd: u64, user_buf_ptr: u64, user_buf_len: u64) -> Result<SyscallReturn> {
    // only suppprt STDOUT now.
    debug!("[syscall][id={}][SYS_WRITE]", SYS_WRITE);
    debug!(
        "fd = {}, user_buf_ptr = 0x{:x}, user_buf_len = 0x{:x}",
        fd, user_buf_ptr, user_buf_len
    );

    if fd == STDOUT || fd == STDERR {
        let mut buffer = vec![0u8; user_buf_len as usize];
        read_bytes_from_user(user_buf_ptr as usize, &mut buffer)?;
        let content = alloc::str::from_utf8(buffer.as_slice())?; // TODO: print content
        if fd == STDOUT {
            print!("{}", content);
        } else {
            print!("{}", content);
        }
        Ok(SyscallReturn::Return(user_buf_len as _))
    } else {
        panic!("Unsupported fd number {}", fd);
    }
}
