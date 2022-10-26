use crate::prelude::*;

use crate::{memory::read_bytes_from_user, syscall::SYS_WRITE};

use super::SyscallResult;

const STDOUT: u64 = 1;
const STDERR: u64 = 2;

pub fn sys_write(fd: u64, user_buf_ptr: u64, user_buf_len: u64) -> SyscallResult {
    // only suppprt STDOUT now.
    debug!("[syscall][id={}][SYS_WRITE]", SYS_WRITE);

    if fd == STDOUT || fd == STDERR {
        let mut buffer = vec![0u8; user_buf_len as usize];
        read_bytes_from_user(user_buf_ptr as usize, &mut buffer);
        let content = alloc::str::from_utf8(buffer.as_slice()).expect("Invalid content"); // TODO: print content
        if fd == STDOUT {
            info!("Message from user mode: {:?}", content);
        } else {
            info!("Error message from user mode: {:?}", content);
        }

        SyscallResult::Return(user_buf_len as _)
    } else {
        panic!("Unsupported fd number {}", fd);
    }
}
